// GET /rest/mempool/info.json
// GET /rest/mempool/contents.json

use super::{check_status, CLIENT};
use crate::{error::Error, NODE_REST_COUNTER};
use bitcoin::Txid;
use fxhash::FxHashSet;
use hyper::body::Buf;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// curl -s http://localhost:8332/rest/mempool/info.json | jq
pub async fn info() -> Result<MempoolInfo, Error> {
    let client = CLIENT.clone();
    let bitcoind_addr = crate::globals::bitcoind_addr();

    let uri = format!("http://{bitcoind_addr}/rest/mempool/info.json").parse()?;
    let resp = client.get(uri).await?;
    NODE_REST_COUNTER
        .with_label_values(&["mempool/info", "json", resp.status().as_str()])
        .inc();
    check_status(resp.status(), Error::RpcMempoolInfo).await?;
    let body_bytes = http_body_util::BodyExt::collect(resp.into_body())
        .await?
        .to_bytes();
    let info: MempoolInfo = serde_json::from_reader(body_bytes.reader())?;
    Ok(info)
}

#[derive(Deserialize)]
pub struct Empty {}

// curl -s http://localhost:8332/rest/mempool/contents.json?verbose=false | jq
pub async fn content(support_verbose: bool) -> Result<FxHashSet<Txid>, Error> {
    let client = CLIENT.clone();
    let bitcoind_addr = crate::globals::bitcoind_addr();

    let uri =
        format!("http://{bitcoind_addr}/rest/mempool/contents.json?verbose={support_verbose}")
            .parse()?;
    let resp = client.get(uri).await?;
    NODE_REST_COUNTER
        .with_label_values(&["mempool/contents", "json", resp.status().as_str()])
        .inc();
    check_status(resp.status(), Error::RpcMempoolContent).await?;
    let body_bytes = http_body_util::BodyExt::collect(resp.into_body())
        .await?
        .to_bytes();

    let content: FxHashSet<Txid> = if support_verbose {
        serde_json::from_reader(body_bytes.reader())?
    } else {
        let content: HashMap<Txid, Empty> = serde_json::from_reader(body_bytes.reader())?;
        content.into_keys().collect()
    };

    Ok(content)
}

pub async fn content_verbose() -> Result<HashMap<Txid, MempoolEntry>, Error> {
    let client = CLIENT.clone();
    let bitcoind_addr = crate::globals::bitcoind_addr();

    let uri = format!("http://{bitcoind_addr}/rest/mempool/contents.json?verbose=true").parse()?;
    let resp = client.get(uri).await?;
    NODE_REST_COUNTER
        .with_label_values(&["mempool/contents/verbose", "json", resp.status().as_str()])
        .inc();
    check_status(resp.status(), Error::RpcMempoolContent).await?;
    let body_bytes = http_body_util::BodyExt::collect(resp.into_body())
        .await?
        .to_bytes();

    Ok(serde_json::from_reader(body_bytes.reader())?)
}

#[derive(Debug, Deserialize)]
pub struct MempoolEntry {
    pub weight: u64,
    pub fees: MempoolFees,
}

#[derive(Debug, Deserialize)]
pub struct MempoolFees {
    /// Base fee in BTC, as returned by Bitcoin Core.
    pub base: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MempoolInfo {
    pub loaded: bool,
    pub size: u32,
    pub bytes: u32,
    pub usage: u64,
    pub total_fee: f64,
    pub maxmempool: u32,
    pub mempoolminfee: f64,
    pub minrelaytxfee: f64,
    pub unbroadcastcount: u32,
}

#[cfg(test)]
mod tests {
    use super::MempoolEntry;

    #[test]
    fn parse_verbose_mempool_entry() {
        let json = r#"{
            "vsize": 126,
            "weight": 501,
            "time": 1776144227,
            "height": 946208,
            "descendantcount": 12,
            "descendantsize": 1512,
            "ancestorcount": 9,
            "ancestorsize": 1134,
            "wtxid": "44952e29a0fb724d3111e66112f2e6ab0378bd4a78b28214d5cba14014f3f6b3",
            "fees": {
              "base": 1.6E-7,
              "modified": 1.6E-7,
              "ancestor": 0.00000144,
              "descendant": 0.00000201
            },
            "depends": [
              "f7e985fafd576c63895a071121832377244366729b581509ed8b11f6b566cedf"
            ],
            "spentby": [
              "dd4e52d964ebbbabe3e5eea4ca854307f4af1807773fd8f7c5fcf8c8ec93b67b"
            ],
            "bip125-replaceable": true,
            "unbroadcast": false
        }"#;

        let entry: MempoolEntry = serde_json::from_str(json).unwrap();

        assert_eq!(entry.weight, 501);
        assert_eq!((entry.fees.base * 100_000_000.0).round() as usize, 16);
    }
}
