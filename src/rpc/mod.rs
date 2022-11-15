use chrono::{DateTime, NaiveDateTime, Utc};
use hyper::{client::HttpConnector, Client};
use once_cell::sync::Lazy;

pub static CLIENT: Lazy<Client<HttpConnector>> = Lazy::new(|| Client::new());

pub mod block;
pub mod blockhashbyheight;
pub mod chaininfo;
pub mod headers;
pub mod mempool;
pub mod tx;
pub mod txout;

fn ts_to_date_time_utc(ts: u32) -> String {
    let ndt = NaiveDateTime::from_timestamp_opt(ts as i64, 0).unwrap();
    let dt = DateTime::<Utc>::from_utc(ndt, Utc);
    dt.format("%Y-%m-%d %H:%M:%S %Z").to_string() // 2022-11-18 07:53:03 UTC
}
