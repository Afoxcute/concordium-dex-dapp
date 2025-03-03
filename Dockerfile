ARG node_base_image=node:20-slim
ARG rust_base_image=rust:1.73

FROM ${node_base_image} AS node_build

WORKDIR /app
COPY package.json ./package.json
COPY yarn.lock ./yarn.lock
COPY tsconfig.json ./tsconfig.json
COPY esbuild.config.ts ./
COPY src ./src

RUN yarn && yarn cache clean
RUN yarn build

FROM ${rust_base_image} AS rust_build
WORKDIR /verifier
COPY ../deps/concordium-rust-sdk /deps/concordium-rust-sdk
COPY verifier/src ./src
COPY verifier/Cargo.lock ./Cargo.lock
COPY verifier/Cargo.toml ./Cargo.toml

RUN sed -i 's|../../deps/concordium-rust-sdk/|/deps/concordium-rust-sdk/|g' Cargo.toml
RUN cargo build --release

FROM ubuntu:22.04
WORKDIR /build

ENV PORT=20000
ENV NODE=http://grpc.testnet.concordium.com
ENV LOG_LEVEL=info
ENV STATEMENT='[{"type":"AttributeInSet","attributeTag":"idDocIssuer","set":["AT","BE","BG","CY","CZ","DK","EE","FI","FR","DE","GR","HU","IE","IT","LV","LT","LU","MT","NL","PL","PT","RO","SK","SI","ES","SE","HR"]},{"type":"AttributeInRange","attributeTag":"dob","lower":"18000101","upper":"20070627"}]'
ENV NAMES='["I Scream", "Starry Night", "Tranquility", "Quiet", "Storm", "Timeless", "Endless Rain"]'

COPY --from=rust_build /verifier/target/release/dex-verifier ./main
COPY --from=node_build /app/public ./public
RUN chmod +x ./main

# Create an entrypoint script to handle environment variable expansion
RUN echo '#!/bin/bash\n\
exec ./main \
  --node "$NODE" \
  --port "$PORT" \
  --log-level "$LOG_LEVEL" \
  --statement "$STATEMENT" \
  --names "$NAMES" \
  --public-folder public' > /build/entrypoint.sh && \
  chmod +x /build/entrypoint.sh

# Use the entrypoint script instead of direct CMD
ENTRYPOINT ["/build/entrypoint.sh"]