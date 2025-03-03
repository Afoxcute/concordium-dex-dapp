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
COPY verifier/config ./config

RUN sed -i 's|../../deps/concordium-rust-sdk/|/deps/concordium-rust-sdk/|g' Cargo.toml
RUN cargo build --release

FROM ${node_base_image}
WORKDIR /app

# Set default environment variables
ENV PORT=20000
ENV NODE=https://grpc.testnet.concordium.com:20000
ENV LOG_LEVEL=info

# Copy necessary files from build stages
COPY --from=rust_build /verifier/target/release/dex-verifier ./dex-verifier
COPY --from=rust_build /verifier/config ./config
COPY --from=node_build /app/public ./public
COPY --from=node_build /app/package.json ./
COPY --from=node_build /app/yarn.lock ./

# Install production dependencies only
RUN yarn install --production && yarn cache clean

# Create an entrypoint script to handle environment variable expansion and file reading
RUN echo '#!/bin/bash\n\
exec yarn start \
  --statement "$(cat config/statement.json)" \
  --names "$(cat config/names.json)" \
  --node "$NODE"' > /app/entrypoint.sh && \
  chmod +x /app/entrypoint.sh

# Use the entrypoint script
ENTRYPOINT ["/app/entrypoint.sh"]