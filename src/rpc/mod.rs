use crate::error::Error;
use chrono::DateTime;
use hyper::StatusCode;
use hyper_util::{client::legacy::Client, rt::TokioExecutor};
use once_cell::sync::Lazy;
use std::time::Duration;

pub static CLIENT: Lazy<
    Client<
        hyper_util::client::legacy::connect::HttpConnector,
        http_body_util::Full<hyper::body::Bytes>,
    >,
> = Lazy::new(|| Client::builder(TokioExecutor::new()).build_http());

pub mod block;
pub mod blockhashbyheight;
pub mod chaininfo;
pub mod headers;
pub mod mempool;
pub mod tx;
pub mod txout;

fn ts_to_date_time_utc(ts: u32) -> String {
    let ndt = DateTime::from_timestamp(ts as i64, 0).unwrap();
    ndt.format("%Y-%m-%d %H:%M:%S %Z").to_string() // 2022-11-18 07:53:03 UTC
}

async fn check_status<F: FnOnce(StatusCode) -> Error>(
    status: StatusCode,
    error: F,
) -> Result<(), Error> {
    if status == 200 {
        Ok(())
    } else {
        let e = error(status);
        log::warn!("status {} error:{:?}", status, e);
        if status == 503 {
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        Err(e)
    }
}
