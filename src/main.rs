use clap::Parser;
use env_logger::Env;
use fbbe::{inner_main, Arguments};

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();
    let args = Arguments::parse();

    if let Err(e) = inner_main(args).await {
        log::error!("{}", e);
    }
}
