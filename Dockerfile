ARG node_base_image=node:20-slim
ARG rust_base_image=rust:1.73

FROM ${node_base_image} AS node_build

WORKDIR /app
COPY ./concordium-dex-dapp/package.json ./package.json
COPY ./concordium-dex-dapp/yarn.lock ./yarn.lock
COPY ./concordium-dex-dapp/tsconfig.json ./tsconfig.json
COPY ./concordium-dex-dapp/esbuild.config.ts ./
COPY ./concordium-dex-dapp/types ./types
COPY ./concordium-dex-dapp/src ./src

RUN yarn && yarn cache clean
RUN yarn build

FROM ${rust_base_image} AS rust_build
WORKDIR /verifier
COPY ./deps/concordium-rust-sdk /deps/concordium-rust-sdk
COPY ./concordium-dex-dapp/verifier/src ./src
COPY ./concordium-dex-dapp/verifier/Cargo.lock ./Cargo.lock
COPY ./concordium-dex-dapp/verifier/Cargo.toml ./Cargo.toml

RUN cargo build --release

FROM ubuntu:22.04
WORKDIR /build

ENV PORT=8100
ENV NODE=http://172.17.0.1:20000
ENV LOG_LEVEL=info
ENV STATEMENT='[{"type":"AttributeInSet","attributeTag":"idDocIssuer","set":["AT","BE","BG","CY","CZ","DK","EE","FI","FR","DE","GR","HU","IE","IT","LV","LT","LU","MT","NL","PL","PT","RO","SK","SI","ES","SE","HR"]},{"type":"AttributeInRange","attributeTag":"dob","lower":"18000101","upper":"20070627"}]'
ENV NAMES='["I Scream", "Starry Night", "Tranquility", "Quiet", "Storm", "Timeless", "Endless Rain"]'

COPY --from=rust_build ./verifier/target/release/dex-verifier ./main
COPY --from=node_build ./app/public ./public
RUN chmod +x ./main

CMD ./main --node "${NODE}" --port "${PORT}" --log-level "${LOG_LEVEL}" --statement "${STATEMENT}" --names "${NAMES}" --public-folder public
