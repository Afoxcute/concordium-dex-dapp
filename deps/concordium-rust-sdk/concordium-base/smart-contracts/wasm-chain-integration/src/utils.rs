//! Various utilities for testing and extraction of schemas and build
//! information.

use crate::{
    v1::{host, trie, EmittedDebugStatement, InstanceState},
    ExecResult,
};
use anyhow::{anyhow, bail, ensure, Context};
pub use concordium_contracts_common::WasmVersion;
use concordium_contracts_common::{
    self as concordium_std, from_bytes, hashes, schema, Cursor, Deserial,
};
use concordium_wasm::{
    artifact::{Artifact, ArtifactNamedImport, RunnableCode, TryFromImport},
    machine::{self, NoInterrupt, Value},
    parse::{parse_custom, parse_skeleton, Skeleton},
    types::{ExportDescription, Module, Name},
    utils,
    validate::{self, ValidationConfig},
};
use rand::{prelude::*, RngCore};
use std::{collections::BTreeMap, default::Default};

/// A host which traps for any function call.
pub struct TrapHost;

impl<I> machine::Host<I> for TrapHost {
    type Interrupt = NoInterrupt;

    fn tick_initial_memory(&mut self, _num_pages: u32) -> machine::RunResult<()> { Ok(()) }

    fn call(
        &mut self,
        _f: &I,
        _memory: &mut [u8],
        _stack: &mut machine::RuntimeStack,
    ) -> machine::RunResult<Option<NoInterrupt>> {
        bail!("TrapHost traps on all host calls.")
    }

    fn tick_energy(&mut self, _energy: u64) -> machine::RunResult<()> { Ok(()) }

    fn track_call(&mut self) -> machine::RunResult<()> { Ok(()) }

    fn track_return(&mut self) {}
}

/// A host which traps for any function call apart from `report_error` which it
/// prints to standard out and `get_random` that calls a random number
/// generator.
pub struct TestHost<'a, R, BackingStore> {
    /// A RNG for randomised testing.
    rng:              Option<R>,
    /// A flag set to `true` if the RNG was used.
    rng_used:         bool,
    /// Debug statements in the order they were emitted.
    pub debug_events: Vec<EmittedDebugStatement>,
    /// In-memory instance state used for state-related host calls.
    state:            InstanceState<'a, BackingStore>,
}

impl<'a, R: RngCore, BackingStore> TestHost<'a, R, BackingStore> {
    /// Create a new `TestHost` instance with the given RNG, set the flag to
    /// unused, no debug events and use the provided instance state for
    /// state-related host function calls.
    pub fn new(rng: R, state: InstanceState<'a, BackingStore>) -> Self {
        TestHost {
            rng: Some(rng),
            rng_used: false,
            debug_events: Vec::new(),
            state,
        }
    }
}

/// Type providing `ValidateImportExport` implementation which only ensure no
/// duplicate imports. Any module name and item name and type is
/// considered valid for both import and export.
pub struct NoDuplicateImport;

impl validate::ValidateImportExport for NoDuplicateImport {
    /// Simply ensure that there are no duplicates.
    #[cfg_attr(not(feature = "fuzz-coverage"), inline(always))]
    fn validate_import_function(
        &self,
        duplicate: bool,
        _mod_name: &Name,
        _item_name: &Name,
        _ty: &concordium_wasm::types::FunctionType,
    ) -> bool {
        !duplicate
    }

    #[cfg_attr(not(feature = "fuzz-coverage"), inline(always))]
    fn validate_export_function(
        &self,
        _item_name: &Name,
        _ty: &concordium_wasm::types::FunctionType,
    ) -> bool {
        true
    }
}

#[derive(Debug, Clone)]
/// An auxiliary datatype used by `report_error` to be able to
/// retain the structured information in case we want to use it later
/// to insert proper links to the file, or other formatting.
pub enum ReportError {
    /// An error reported by `report_error`
    Reported {
        filename: String,
        line:     u32,
        column:   u32,
        msg:      String,
    },
    /// Some other source of error. We only have the description, and no
    /// location.
    Other {
        msg: String,
    },
}

impl std::fmt::Display for ReportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReportError::Reported {
                filename,
                line,
                column,
                msg,
            } => write!(f, "{}, {}:{}:{}", msg, filename, line, column),
            ReportError::Other {
                msg,
            } => msg.fmt(f),
        }
    }
}

/// Extract debug information from the memory and stack. This is used when
/// handling `report_error` and `debug_print` functions.
pub(crate) fn extract_debug(
    memory: &mut [u8],
    stack: &mut machine::RuntimeStack,
) -> anyhow::Result<(String, u32, u32, String)> {
    let column = unsafe { stack.pop_u32() };
    let line = unsafe { stack.pop_u32() };
    let filename_length = unsafe { stack.pop_u32() } as usize;
    let filename_start = unsafe { stack.pop_u32() } as usize;
    let msg_length = unsafe { stack.pop_u32() } as usize;
    let msg_start = unsafe { stack.pop_u32() } as usize;
    ensure!(filename_start + filename_length <= memory.len(), "Illegal memory access.");
    ensure!(msg_start + msg_length <= memory.len(), "Illegal memory access.");
    let msg = std::str::from_utf8(&memory[msg_start..msg_start + msg_length])?.to_owned();
    let filename =
        std::str::from_utf8(&memory[filename_start..filename_start + filename_length])?.to_owned();
    Ok((filename, line, column, msg))
}

impl<'a, R: RngCore, BackingStore: trie::BackingStoreLoad> machine::Host<ArtifactNamedImport>
    for TestHost<'a, R, BackingStore>
{
    type Interrupt = NoInterrupt;

    fn tick_initial_memory(&mut self, _num_pages: u32) -> machine::RunResult<()> {
        // The test host does not count energy.
        Ok(())
    }

    fn call(
        &mut self,
        f: &ArtifactNamedImport,
        memory: &mut [u8],
        stack: &mut machine::RuntimeStack,
    ) -> machine::RunResult<Option<NoInterrupt>> {
        // We don't track the energy usage in this host, so to reuse code which does, we
        // provide a really large amount of energy to preventing the case of
        // running out of energy.
        let mut energy = crate::InterpreterEnergy::new(u64::MAX);
        if f.matches("concordium", "report_error") {
            let (filename, line, column, msg) = extract_debug(memory, stack)?;
            bail!(ReportError::Reported {
                filename,
                line,
                column,
                msg
            })
        } else if f.matches("concordium", "get_random") {
            let size = unsafe { stack.pop_u32() } as usize;
            let dest = unsafe { stack.pop_u32() } as usize;
            ensure!(dest + size <= memory.len(), "Illegal memory access.");
            self.rng_used = true;
            match self.rng.as_mut() {
                Some(r) => {
                    r.try_fill_bytes(&mut memory[dest..dest + size])?;
                }
                None => {
                    bail!("Expected an initialized RNG.");
                }
            }
        } else if f.matches("concordium", "debug_print") {
            let (filename, line, column, msg) = extract_debug(memory, stack)?;
            self.debug_events.push(EmittedDebugStatement {
                filename,
                line,
                column,
                msg,
                remaining_energy: 0.into(), // debug host does not have energy.
            });
        } else if f.matches("concordium", "state_lookup_entry") {
            host::state_lookup_entry(memory, stack, &mut energy, &mut self.state)?;
        } else if f.matches("concordium", "state_create_entry") {
            host::state_create_entry(memory, stack, &mut energy, &mut self.state)?;
        } else if f.matches("concordium", "state_delete_entry") {
            host::state_delete_entry(memory, stack, &mut energy, &mut self.state)?;
        } else if f.matches("concordium", "state_delete_prefix") {
            host::state_delete_prefix(memory, stack, &mut energy, &mut self.state)?;
        } else if f.matches("concordium", "state_iterate_prefix") {
            host::state_iterator(memory, stack, &mut energy, &mut self.state)?;
        } else if f.matches("concordium", "state_iterator_next") {
            host::state_iterator_next(stack, &mut energy, &mut self.state)?;
        } else if f.matches("concordium", "state_iterator_delete") {
            host::state_iterator_delete(stack, &mut energy, &mut self.state)?;
        } else if f.matches("concordium", "state_iterator_key_size") {
            host::state_iterator_key_size(stack, &mut energy, &mut self.state)?;
        } else if f.matches("concordium", "state_iterator_key_read") {
            host::state_iterator_key_read(memory, stack, &mut energy, &mut self.state)?;
        } else if f.matches("concordium", "state_entry_read") {
            host::state_entry_read(memory, stack, &mut energy, &mut self.state)?;
        } else if f.matches("concordium", "state_entry_write") {
            host::state_entry_write(memory, stack, &mut energy, &mut self.state)?;
        } else if f.matches("concordium", "state_entry_size") {
            host::state_entry_size(stack, &mut energy, &mut self.state)?;
        } else if f.matches("concordium", "state_entry_resize") {
            host::state_entry_resize(stack, &mut energy, &mut self.state)?;
        } else {
            bail!("Unsupported host function call.")
        }
        Ok(None)
    }

    fn tick_energy(&mut self, _energy: u64) -> machine::RunResult<()> { Ok(()) }

    fn track_call(&mut self) -> machine::RunResult<()> { Ok(()) }

    fn track_return(&mut self) {}
}

/// The type of results returned after running a test.
pub struct TestResult {
    /// The name of the test that is being reported.
    pub test_name:    String,
    /// The result of the test. [`None`] if the test passed.
    /// In case of failure the `bool` flag indicates whether randomness was used
    /// or not.
    pub result:       Option<(ReportError, bool)>,
    /// Any debug events emitted as part of the test.
    pub debug_events: Vec<EmittedDebugStatement>,
}

/// Instantiates the module with an external function to report back errors and
/// a seed that is used to instantiate a RNG for randomized testing. Then tries
/// to run exported test-functions, which are present if compiled with
/// the wasm-test feature.
///
/// The return value is a list of test results.
pub fn run_module_tests(module_bytes: &[u8], seed: u64) -> ExecResult<Vec<TestResult>> {
    let artifact = utils::instantiate::<ArtifactNamedImport, _>(
        ValidationConfig::V1,
        &NoDuplicateImport,
        module_bytes,
    )?
    .artifact;
    let mut out = Vec::with_capacity(artifact.export.len());
    for name in artifact.export.keys() {
        if let Some(test_name) = name.as_ref().strip_prefix("concordium_test ") {
            // create a `TestHost` instance for each test with the usage flag set to `false`
            let mut initial_state = trie::MutableState::initial_state();
            let mut loader = trie::Loader::new(Vec::new());
            let mut test_host = {
                let inner = initial_state.get_inner(&mut loader);
                let state = InstanceState::new(loader, inner);
                TestHost::new(SmallRng::seed_from_u64(seed), state)
            };
            let res = artifact.run(&mut test_host, name, &[]);
            match res {
                Ok(_) => {
                    let result = TestResult {
                        test_name:    test_name.to_owned(),
                        result:       None,
                        debug_events: test_host.debug_events,
                    };
                    out.push(result);
                }
                Err(msg) => {
                    if let Some(err) = msg.downcast_ref::<ReportError>() {
                        let result = TestResult {
                            test_name:    test_name.to_owned(),
                            result:       Some((err.clone(), test_host.rng_used)),
                            debug_events: test_host.debug_events,
                        };
                        out.push(result);
                    } else {
                        let result = TestResult {
                            test_name:    test_name.to_owned(),
                            result:       Some((
                                ReportError::Other {
                                    msg: msg.to_string(),
                                },
                                test_host.rng_used,
                            )),
                            debug_events: test_host.debug_events,
                        };
                        out.push(result);
                    }
                }
            };
        }
    }
    Ok(out)
}

/// Tries to generate a state schema and schemas for parameters of methods of a
/// V0 contract.
pub fn generate_contract_schema_v0(
    module_bytes: &[u8],
) -> ExecResult<schema::VersionedModuleSchema> {
    let artifact = utils::instantiate::<ArtifactNamedImport, _>(
        ValidationConfig::V0,
        &NoDuplicateImport,
        module_bytes,
    )?
    .artifact;

    let mut contract_schemas = BTreeMap::new();

    for name in artifact.export.keys() {
        if let Some(contract_name) = name.as_ref().strip_prefix("concordium_schema_state_") {
            let schema_type = generate_schema_run(&artifact, name.as_ref())?;

            // Get the mutable reference to the contract schema, or make a new empty one if
            // an entry does not yet exist.
            let contract_schema = contract_schemas
                .entry(contract_name.to_owned())
                .or_insert_with(schema::ContractV0::default);

            contract_schema.state = Some(schema_type);
        } else if let Some(rest) = name.as_ref().strip_prefix("concordium_schema_function_") {
            if let Some(contract_name) = rest.strip_prefix("init_") {
                let schema_type = generate_schema_run(&artifact, name.as_ref())?;

                let contract_schema = contract_schemas
                    .entry(contract_name.to_owned())
                    .or_insert_with(schema::ContractV0::default);
                contract_schema.init = Some(schema_type);
            } else if rest.contains('.') {
                let schema_type = generate_schema_run(&artifact, name.as_ref())?;

                // Generates receive-function parameter schema type
                let split_name: Vec<_> = rest.splitn(2, '.').collect();
                let contract_name = split_name[0];
                let function_name = split_name[1];

                let contract_schema = contract_schemas
                    .entry(contract_name.to_owned())
                    .or_insert_with(schema::ContractV0::default);

                contract_schema.receive.insert(function_name.to_owned(), schema_type);
            } else {
                // do nothing, some other function that is neither init nor
                // receive.
            }
        }
    }

    Ok(schema::VersionedModuleSchema::V0(schema::ModuleV0 {
        contracts: contract_schemas,
    }))
}

/// Tries to generate schemas for parameters and return values of methods for a
/// contract with a V1 schema.
pub fn generate_contract_schema_v1(
    module_bytes: &[u8],
) -> ExecResult<schema::VersionedModuleSchema> {
    let artifact = utils::instantiate::<ArtifactNamedImport, _>(
        ValidationConfig::V1,
        &NoDuplicateImport,
        module_bytes,
    )?
    .artifact;

    let mut contract_schemas = BTreeMap::new();

    for name in artifact.export.keys() {
        if let Some(rest) = name.as_ref().strip_prefix("concordium_schema_function_") {
            if let Some(contract_name) = rest.strip_prefix("init_") {
                let function_schema = generate_schema_run(&artifact, name.as_ref())?;

                let contract_schema = contract_schemas
                    .entry(contract_name.to_owned())
                    .or_insert_with(schema::ContractV1::default);
                contract_schema.init = Some(function_schema);
            } else if rest.contains('.') {
                let function_schema = generate_schema_run(&artifact, name.as_ref())?;

                // Generates receive-function parameter schema type
                let split_name: Vec<_> = rest.splitn(2, '.').collect();
                let contract_name = split_name[0];
                let function_name = split_name[1];

                let contract_schema = contract_schemas
                    .entry(contract_name.to_owned())
                    .or_insert_with(schema::ContractV1::default);

                contract_schema.receive.insert(function_name.to_owned(), function_schema);
            } else {
                // do nothing, some other function that is neither init nor
                // receive.
            }
        }
    }

    Ok(schema::VersionedModuleSchema::V1(schema::ModuleV1 {
        contracts: contract_schemas,
    }))
}

/// Tries to generate schemas for parameters and return values of methods for a
/// contract with a V2 schema.
pub fn generate_contract_schema_v2(
    module_bytes: &[u8],
) -> ExecResult<schema::VersionedModuleSchema> {
    let artifact = utils::instantiate::<ArtifactNamedImport, _>(
        ValidationConfig::V1,
        &NoDuplicateImport,
        module_bytes,
    )?
    .artifact;

    let mut contract_schemas = BTreeMap::new();

    for name in artifact.export.keys() {
        if let Some(rest) = name.as_ref().strip_prefix("concordium_schema_function_") {
            if let Some(contract_name) = rest.strip_prefix("init_") {
                let function_schema = generate_schema_run(&artifact, name.as_ref())?;

                let contract_schema = contract_schemas
                    .entry(contract_name.to_owned())
                    .or_insert_with(schema::ContractV2::default);
                contract_schema.init = Some(function_schema);
            } else if rest.contains('.') {
                let function_schema = generate_schema_run(&artifact, name.as_ref())?;

                // Generates receive-function parameter schema type
                let split_name: Vec<_> = rest.splitn(2, '.').collect();
                let contract_name = split_name[0];
                let function_name = split_name[1];

                let contract_schema = contract_schemas
                    .entry(contract_name.to_owned())
                    .or_insert_with(schema::ContractV2::default);

                contract_schema.receive.insert(function_name.to_owned(), function_schema);
            } else {
                // do nothing, some other function that is neither init nor
                // receive.
            }
        }
    }

    Ok(schema::VersionedModuleSchema::V2(schema::ModuleV2 {
        contracts: contract_schemas,
    }))
}

/// Tries to generate schemas for events, parameters, return values, and errors
/// of methods for a contract with a V3 schema.
pub fn generate_contract_schema_v3(
    module_bytes: &[u8],
) -> ExecResult<schema::VersionedModuleSchema> {
    let artifact = utils::instantiate::<ArtifactNamedImport, _>(
        ValidationConfig::V1,
        &NoDuplicateImport,
        module_bytes,
    )?
    .artifact;

    let mut contract_schemas = BTreeMap::new();

    for name in artifact.export.keys() {
        if let Some(rest) = name.as_ref().strip_prefix("concordium_event_schema_") {
            if let Some(contract_name) = rest.strip_prefix("init_") {
                // Generate event schema
                let function_schema_event = generate_schema_run(&artifact, name.as_ref())?;

                let contract_schema = contract_schemas
                    .entry(contract_name.to_owned())
                    .or_insert_with(schema::ContractV3::default);
                contract_schema.event = Some(function_schema_event);
            }
            // The event schema attached to the init function is globally
            // available in the smart contract and is applied to all
            // events logged by receive/init functions. There is no
            // need to create a separate event schema for receive functions.
        } else if let Some(rest) = name.as_ref().strip_prefix("concordium_schema_function_") {
            if let Some(contract_name) = rest.strip_prefix("init_") {
                // Generate init-function schema
                let function_schema = generate_schema_run(&artifact, name.as_ref())?;

                let contract_schema = contract_schemas
                    .entry(contract_name.to_owned())
                    .or_insert_with(schema::ContractV3::default);
                contract_schema.init = Some(function_schema);
            } else if rest.contains('.') {
                // Generate receive-function schema
                let function_schema = generate_schema_run(&artifact, name.as_ref())?;

                let split_name: Vec<_> = rest.splitn(2, '.').collect();
                let contract_name = split_name[0];
                let function_name = split_name[1];

                let contract_schema = contract_schemas
                    .entry(contract_name.to_owned())
                    .or_insert_with(schema::ContractV3::default);

                contract_schema.receive.insert(function_name.to_owned(), function_schema);
            } else {
                // do nothing: no event schema and not a schema that was
                // attached to an init/ receive function
            }
        }
    }

    Ok(schema::VersionedModuleSchema::V3(schema::ModuleV3 {
        contracts: contract_schemas,
    }))
}

/// Runs the given schema function and reads the resulting function schema from
/// memory, attempting to parse it. If this fails, an error is returned.
fn generate_schema_run<I: TryFromImport, C: RunnableCode, SchemaType: Deserial>(
    artifact: &Artifact<I, C>,
    schema_fn_name: &str,
) -> ExecResult<SchemaType> {
    let (ptr, memory) = if let machine::ExecutionOutcome::Success {
        result: Some(Value::I32(ptr)),
        memory,
    } = artifact.run(&mut TrapHost, schema_fn_name, &[])?
    {
        (ptr as u32 as usize, memory)
    } else {
        bail!("Schema derivation function is malformed.")
    };

    // First we read an u32 which is the length of the serialized schema
    ensure!(ptr + 4 <= memory.len(), "Illegal memory access.");
    let len = u32::deserial(&mut Cursor::new(&memory[ptr..ptr + 4]))
        .map_err(|_| anyhow!("Cannot read schema length."))?;

    // Read the schema with offset of the u32
    ensure!(ptr + 4 + len as usize <= memory.len(), "Illegal memory access when reading schema.");
    let schema_bytes = &memory[ptr + 4..ptr + 4 + len as usize];
    SchemaType::deserial(&mut Cursor::new(schema_bytes))
        .map_err(|_| anyhow!("Failed deserialising the schema."))
}

/// Get the init methods of the module.
pub fn get_inits(module: &Module) -> Vec<&Name> {
    let mut out = Vec::new();
    for export in module.export.exports.iter() {
        if export.name.as_ref().starts_with("init_") && !export.name.as_ref().contains('.') {
            if let ExportDescription::Func {
                ..
            } = export.description
            {
                out.push(&export.name);
            }
        }
    }
    out
}

/// Get the receive methods of the module.
pub fn get_receives(module: &Module) -> Vec<&Name> {
    let mut out = Vec::new();
    for export in module.export.exports.iter() {
        if export.name.as_ref().contains('.') {
            if let ExportDescription::Func {
                ..
            } = export.description
            {
                out.push(&export.name);
            }
        }
    }
    out
}

/// Get the embedded schema for smart contract modules version 0 if it exists.
///
/// First attempt to use the schema in the custom section "concordium-schema"
/// and if this is not present try to use the custom section
/// "concordium-schema-v1".
pub fn get_embedded_schema_v0(bytes: &[u8]) -> ExecResult<schema::VersionedModuleSchema> {
    let skeleton = parse_skeleton(bytes)?;
    let mut schema_v1_section = None;
    let mut schema_versioned_section = None;
    for ucs in skeleton.custom.iter() {
        let cs = parse_custom(ucs)?;

        if cs.name.as_ref() == "concordium-schema" && schema_versioned_section.is_none() {
            schema_versioned_section = Some(cs)
        } else if cs.name.as_ref() == "concordium-schema-v1" && schema_v1_section.is_none() {
            schema_v1_section = Some(cs)
        }
    }

    if let Some(cs) = schema_versioned_section {
        let module: schema::VersionedModuleSchema =
            from_bytes(cs.contents).map_err(|_| anyhow!("Failed parsing schema"))?;
        Ok(module)
    } else if let Some(cs) = schema_v1_section {
        let module = from_bytes(cs.contents).map_err(|_| anyhow!("Failed parsing schema"))?;
        Ok(schema::VersionedModuleSchema::V0(module))
    } else {
        bail!("No schema found in the module")
    }
}

/// Get the embedded schema for smart contract modules version 1 if it exists.
///
/// First attempt to use the schema in the custom section "concordium-schema"
/// and if this is not present try to use the custom section
/// "concordium-schema-v2".
pub fn get_embedded_schema_v1(bytes: &[u8]) -> ExecResult<schema::VersionedModuleSchema> {
    let skeleton = parse_skeleton(bytes)?;
    let mut schema_v2_section = None;
    let mut schema_versioned_section = None;
    for ucs in skeleton.custom.iter() {
        let cs = parse_custom(ucs)?;
        if cs.name.as_ref() == "concordium-schema" && schema_versioned_section.is_none() {
            schema_versioned_section = Some(cs)
        } else if cs.name.as_ref() == "concordium-schema-v2" && schema_v2_section.is_none() {
            schema_v2_section = Some(cs)
        }
    }

    if let Some(cs) = schema_versioned_section {
        let module: schema::VersionedModuleSchema =
            from_bytes(cs.contents).map_err(|_| anyhow!("Failed parsing schema"))?;
        Ok(module)
    } else if let Some(cs) = schema_v2_section {
        let module = from_bytes(cs.contents).map_err(|_| anyhow!("Failed parsing schema"))?;
        Ok(schema::VersionedModuleSchema::V1(module))
    } else {
        bail!("No schema found in the module")
    }
}

/// The build information that will be embedded as a custom section to
/// support reproducible builds.
#[derive(Debug, Clone, concordium_contracts_common::Serialize)]
pub struct BuildInfo {
    /// The SHA256 hash of the tar file used to build.
    /// Note that this is the hash of the **tar** file alone, not of any
    /// compressed version.
    pub archive_hash:  hashes::Hash,
    /// The link to where the source code will be located.
    pub source_link:   Option<String>,
    /// The build image that was used.
    pub image:         String,
    /// The exact command invocation inside the image that was used to produce
    /// the contract.
    pub build_command: Vec<String>,
}

/// A versioned build information. This is the information that is embedded in a
/// custom section. Currently there is one version, but the format supports
/// future evolution.
///
/// The value is embedded in a custom section serialized using the smart
/// contract serialization format
/// ([`Serial`](concordium_contracts_common::Serial) trait).
#[derive(Debug, Clone, concordium_contracts_common::Serialize)]
pub enum VersionedBuildInfo {
    V0(BuildInfo),
}

/// Name of the custom section that contains the build information of the
/// module.
pub const BUILD_INFO_SECTION_NAME: &str = "concordium-build-info";

#[derive(Debug, thiserror::Error)]
pub enum CustomSectionLookupError {
    #[error("Custom section with a provided name is not present.")]
    Missing,
    #[error("Multiple custom sections with the given name are present.")]
    Multiple,
    #[error("Parse error: {0}.")]
    MalformedData(#[from] anyhow::Error),
}

impl CustomSectionLookupError {
    /// Returns whether the value is of the [`Missing`](Self::Missing) variant.
    pub fn is_missing(&self) -> bool { matches!(self, Self::Missing) }
}

/// Extract the embedded [`VersionedBuildInfo`] from a Wasm module.
pub fn get_build_info(bytes: &[u8]) -> Result<VersionedBuildInfo, CustomSectionLookupError> {
    let skeleton = parse_skeleton(bytes)?;
    get_build_info_from_skeleton(&skeleton)
}

/// Extract the embedded [`VersionedBuildInfo`] from a [`Skeleton`].
pub fn get_build_info_from_skeleton(
    skeleton: &Skeleton,
) -> Result<VersionedBuildInfo, CustomSectionLookupError> {
    let mut build_context_section = None;
    for ucs in skeleton.custom.iter() {
        let cs = parse_custom(ucs)?;
        if cs.name.as_ref() == BUILD_INFO_SECTION_NAME
            && build_context_section.replace(cs).is_some()
        {
            return Err(CustomSectionLookupError::Multiple);
        }
    }
    let Some(cs) = build_context_section else {
        return Err(CustomSectionLookupError::Missing);
    };
    let info: VersionedBuildInfo = from_bytes(cs.contents).context("Failed parsing build info")?;
    Ok(info)
}

#[cfg(test)]
/// Tests for schema parsing functions.
mod tests {

    #[test]
    fn test_schema_embeddings() {
        let data =
            std::fs::read("../testdata/schemas/cis1-wccd-embedded-schema-v0-unversioned.wasm")
                .expect("Could not read file.");
        if let Err(e) = super::get_embedded_schema_v0(&data) {
            panic!("Failed to parse unversioned v0 module schema: {}", e);
        }

        let data =
            std::fs::read("../testdata/schemas/cis2-wccd-embedded-schema-v1-unversioned.wasm.v1")
                .expect("Could not read file.");
        if let Err(e) = super::get_embedded_schema_v1(&data[8..]) {
            panic!("Failed to parse unversioned v1 module schema: {}", e);
        }

        let data =
            std::fs::read("../testdata/schemas/cis1-wccd-embedded-schema-v0-versioned.wasm.v0")
                .expect("Could not read file.");
        if let Err(e) = super::get_embedded_schema_v0(&data[8..]) {
            panic!("Failed to parse versioned v0 module schema: {}", e);
        }

        let data =
            std::fs::read("../testdata/schemas/cis2-wccd-embedded-schema-v1-versioned.wasm.v1")
                .expect("Could not read file.");
        if let Err(e) = super::get_embedded_schema_v1(&data[8..]) {
            panic!("Failed to parse versioned v1 module schema: {}", e);
        }
    }
}
