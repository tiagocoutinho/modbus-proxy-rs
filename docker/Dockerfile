FROM rust:1.57-slim-buster as build

COPY . /modbus-proxy-rs
WORKDIR /modbus-proxy-rs

RUN cargo build --release

# our final base
FROM rust:1.57-slim-buster

# copy the build artifact from the build stage
COPY --from=build /modbus-proxy-rs/target/release/modbus-proxy-rs .

# set the startup command to run your binary
ENTRYPOINT ["./modbus-proxy-rs"]