ARG node_base_image=node:20-slim
ARG rust_base_image=rust:1.73

# Node.js build stage
FROM ${node_base_image} AS node_build

WORKDIR /app
COPY package.json ./package.json
COPY yarn.lock ./yarn.lock
COPY tsconfig.json ./tsconfig.json
COPY esbuild.config.ts ./
COPY src ./src

# Install dependencies and build frontend
RUN yarn && yarn cache clean
RUN yarn build

# Rust build stage
FROM ${rust_base_image} AS rust_build

# Set up workspace for Rust build
WORKDIR /build
# Copy deps directory first
COPY deps/concordium-rust-sdk /build/deps/concordium-rust-sdk
# Copy verifier directory
COPY verifier /build/verifier

# Build the verifier
WORKDIR /build/verifier
RUN sed -i 's|../../deps/concordium-rust-sdk/|/build/deps/concordium-rust-sdk/|g' Cargo.toml
#RUN cargo build --release

# Final stage
FROM ${node_base_image}
WORKDIR /app

# Set default environment variables
ENV PORT=8100
ENV NODE=https://grpc.testnet.concordium.com:20000
ENV LOG_LEVEL=info

# Copy necessary files from build stages
COPY --from=rust_build /build/verifier/target/release/dex-verifier ./dex-verifier
COPY --from=rust_build /build/verifier/config ./config
COPY --from=node_build /app/public ./public
COPY --from=node_build /app/package.json ./
COPY --from=node_build /app/yarn.lock ./

# Install production dependencies only
RUN yarn install --production && yarn cache clean

# Make sure the verifier is executable
RUN chmod +x ./dex-verifier

# Create an entrypoint script that matches the README's run command
RUN echo '#!/bin/bash\n\
exec yarn start \
  --statement "$(cat config/statement.json)" \
  --names "$(cat config/names.json)" \
  --node "$NODE" \
  --port "$PORT"' > /app/entrypoint.sh && \
  chmod +x /app/entrypoint.sh

# Expose the correct port as mentioned in README
EXPOSE 8100

# Use the entrypoint script
ENTRYPOINT ["/app/entrypoint.sh"]