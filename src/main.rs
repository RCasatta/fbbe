use clap::Parser;
use env_logger::Env;
use fbbe::{inner_main, Arguments};
use std::io::Write;

#[cfg(not(target_env = "msvc"))]
use jemallocator::Jemalloc;

#[cfg(not(target_env = "msvc"))]
#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

#[tokio::main]
async fn main() {
    let mut builder = env_logger::Builder::from_env(Env::default().default_filter_or("info"));
    match std::env::var("RUST_LOG_STYLE") {
        Ok(s) if s == "SYSTEMD" => builder
            .format(|buf, record| {
                writeln!(
                    buf,
                    "<{}>{}: {}",
                    match record.level() {
                        log::Level::Error => 3,
                        log::Level::Warn => 4,
                        log::Level::Info => 6,
                        log::Level::Debug => 7,
                        log::Level::Trace => 7,
                    },
                    record.target(),
                    record.args()
                )
            })
            .init(),
        _ => (),
    };

    builder.init();
    let args = Arguments::parse();

    if let Err(e) = inner_main(args).await {
        log::error!("{}", e);
    }
}
