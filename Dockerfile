FROM rust:1.57-slim-bullseye as build

# 1. Create a new empty shell project
RUN USER=root cargo new --bin modbus-proxy-rs
WORKDIR /modbus-proxy-rs

# 2. Copy our manifests
COPY ./Cargo.lock ./Cargo.lock
COPY ./Cargo.toml ./Cargo.toml

# 3. Build only the dependencies to cache them
RUN cargo build --release
RUN rm src/*.rs

# 4. Now that the dependency is built, copy your source code
COPY ./src ./src

# build for release
RUN rm ./target/release/deps/modbus_proxy_rs*
RUN cargo build --release

# our final base
FROM debian:bullseye-slim

# copy the build artifact from the build stage
COPY --from=build /modbus-proxy-rs/target/release/modbus-proxy-rs .

# set the startup command to run your binary
ENTRYPOINT ["./modbus-proxy-rs"]