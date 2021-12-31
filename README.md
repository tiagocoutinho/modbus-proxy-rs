# Modbus TCP proxy

Many modbus devices support only one or very few clients. This proxy acts as a
bridge between the client and the modbus device. It can be seen as a layer 7
reverse proxy.
This allows multiple clients to communicate with the same modbus device.

When multiple clients are connected, cross messages are avoided by serializing communication on a first come first served REQ/REP basis.

This project is the [Rust][rust] version of the [Python][python] based
[modbus-proxy][modbus-proxy-py] project.

I did it because it fitted my personal goal of exercising with the [Rust][rust]
programming language and it's async based [tokio] library.

The goal was to produce a robust, highly concurrent server with a low
memory footprint.

## Launch

```bash
cargo run --release -- -b 0:5020 --modbus plc1.acme.org:502
```

## Docker

This project ships with a [Dockerfile](./Dockerfile) which you can use as a
base to launch modbus-proxy inside a docker container.

First, build the docker image with:

```bash
$ docker build -t modbus-proxy-rs .
```

To bridge a single modbus device without needing a configuration file is
straight forward:

```bash
$ docker run --init -d -p 5020:502 modbus-proxy-rs -b 0:502 --modbus plc1.acme.org:502
```

Now you should be able to access your modbus device through the modbus-proxy by
connecting your client(s) to `<your-hostname/ip>:5020`.


## Credits

### Development Lead

* Tiago Coutinho <coutinhotiago@gmail.com>

### Contributors

None yet. Why not be the first?

[rust]: https://www.rust-lang.org/
[python]: https://python.org/
[modbus-proxy-py]: https://github.com/tiagocoutinho/modbus-proxy
[tokio]: https://tokio.rs/
