FROM rust:1.57 as build

# 1. Prepare to build using musl
RUN rustup target add x86_64-unknown-linux-musl
RUN apt update && apt install -y musl-tools musl-dev
RUN update-ca-certificates

# 2. setup app user
ENV USER=guest
ENV UID=10001

RUN adduser \
    --disabled-password \
    --gecos "" \
    --home "/nonexistent" \
    --shell "/sbin/nologin" \
    --no-create-home \
    --uid "${UID}" \
    "${USER}"

# 3. Create a new empty shell project
RUN cargo new --bin modbus-proxy-rs
WORKDIR /modbus-proxy-rs

# 4. Copy our manifests
COPY ./Cargo.lock ./Cargo.lock
COPY ./Cargo.toml ./Cargo.toml

# 5. Build only the dependencies to cache them
RUN cargo build --target x86_64-unknown-linux-musl --release
RUN rm src/*.rs

# 6. Now that the dependency is built, copy your source code
COPY ./src ./src

# 7. Build for release
RUN rm ./target/x86_64-unknown-linux-musl/release/deps/modbus_proxy_rs*
RUN cargo build --target x86_64-unknown-linux-musl --release

# 8. Strip debug symbols to reduce binary size
RUN strip -s /modbus-proxy-rs/target/x86_64-unknown-linux-musl/release/modbus-proxy-rs

# our final base
FROM scratch

# Import user definition from builder.
COPY --from=build /etc/passwd /etc/passwd
COPY --from=build /etc/group /etc/group

# Run as guest user
USER guest:guest

# Copy the build artifact from the build stage
COPY --from=build /modbus-proxy-rs/target/x86_64-unknown-linux-musl/release/modbus-proxy-rs .

# Set the startup command to run
ENTRYPOINT ["./modbus-proxy-rs"]
