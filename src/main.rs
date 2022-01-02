use structopt::StructOpt;

use modbus_proxy_rs::Server;

#[derive(Debug, StructOpt)]
#[structopt(
    name = "modbus proxy",
    about = "Connect multiple clients to modbus devices"
)]
struct CmdLine {
    #[structopt(
        short = "c",
        long = "config-file",
        help = "configuration file (accepts YAML, TOML, JSON)"
    )]
    config_file: String,
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    env_logger::init();
    let args = CmdLine::from_args();
    if let Err(error) = Server::launch(&args.config_file).await {
        eprintln!("Configuration error: {}", error)
    }
}
