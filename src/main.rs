use futures::future::join_all;
use structopt::StructOpt;

use modbus_proxy_rs::{bridge_task, Settings};

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
    match Settings::new(&args.config_file) {
        Ok(mut cfg) => {
            let tasks = cfg
                .devices
                .drain(..)
                .map(|device| tokio::spawn(bridge_task(device)))
                .collect::<Vec<_>>();
            join_all(tasks).await;
        }
        Err(error) => {
            eprintln!("Configuration error: {}", error)
        }
    }
}
