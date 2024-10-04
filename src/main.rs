use clap::Parser;
use env_logger::Env;
use fbbe::{inner_main, Arguments};
use std::io::Write;

#[tokio::main]
async fn main() {
    let mut builder = env_logger::Builder::from_env(Env::default().default_filter_or("info"));
    if std::env::var("LOG_AVOID_TIMESTAMP").is_ok() {
        builder.format(|buf, r| writeln!(buf, "{:5} {} {}", r.level(), r.target(), r.args()));
    }

    builder.init();
    let args = Arguments::parse();

    if let Err(e) = inner_main(args).await {
        log::error!("{}", e);
    }
}
