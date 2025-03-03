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

# Install SSL certificates and other dependencies
RUN apt-get update && \
    apt-get install -y \
    curl \
    ca-certificates \
    && update-ca-certificates

# First, copy the SDK
COPY deps/concordium-rust-sdk /build/deps/concordium-rust-sdk

# Install yarn
RUN curl -sS https://dl.yarnpkg.com/debian/pubkey.gpg | apt-key add - && \
    echo "deb https://dl.yarnpkg.com/debian/ stable main" | tee /etc/apt/sources.list.d/yarn.list && \
    apt-get update && \
    apt-get install -y yarn

# Copy verifier directory and package files
COPY package.json yarn.lock ./
COPY verifier ./verifier

# Build the verifier
WORKDIR /build/verifier
RUN cargo build --release
RUN mkdir -p target/release && cp target/release/dex-verifier /build/dex-verifier

# Final stage
FROM ${node_base_image}
WORKDIR /app

# Install SSL certificates in the final stage
RUN apt-get update && \
    apt-get install -y \
    ca-certificates \
    && update-ca-certificates

# Set default environment variables
ENV PORT=8100
ENV NODE=https://grpc.testnet.concordium.com:20000
ENV LOG_LEVEL=info
ENV SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt
ENV SSL_CERT_DIR=/etc/ssl/certs

# Copy necessary files from build stages
COPY --from=rust_build /build/dex-verifier ./dex-verifier
COPY --from=rust_build /build/verifier/config ./config
COPY --from=node_build /app/public ./public
COPY --from=node_build /app/package.json ./package.json
COPY --from=node_build /app/yarn.lock ./yarn.lock

# Install production dependencies only
RUN yarn install --production && yarn cache clean

# Make sure the verifier is executable
RUN chmod +x ./dex-verifier

# Create start script with environment variables
RUN echo '#!/bin/sh\n\
export SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt\n\
export SSL_CERT_DIR=/etc/ssl/certs\n\
./dex-verifier \
  --statement "$(cat config/statement.json)" \
  --names "$(cat config/names.json)" \
  --node "$NODE" \
  --port "$PORT"' > start.sh && \
chmod +x start.sh

# Update package.json to use the correct path
RUN sed -i 's|"start": "./verifier/target/release/dex-verifier"|"start": "./dex-verifier"|' package.json

# Expose the correct port as mentioned in README
EXPOSE 8100

# Use the start script as entrypoint
ENTRYPOINT ["./start.sh"]