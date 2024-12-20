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
        .with_label_values(&["mempool/info", "json"])
        .inc();
    check_status(resp.status(), Error::RpcMempoolInfo).await?;
    let body_bytes = hyper::body::to_bytes(resp.into_body()).await?;
    let info: MempoolInfo = serde_json::from_reader(body_bytes.reader())?;
    Ok(info)
}

#[derive(Deserialize)]
pub struct Empty {}

// curl -s http://localhost:8332/rest/mempool/contents.json?verbose=false | jq
pub async fn content(support_verbose: bool) -> Result<FxHashSet<Txid>, Error> {
    let client = CLIENT.clone();
    let bitcoind_addr = crate::globals::bitcoind_addr();

    let uri = format!("http://{bitcoind_addr}/rest/mempool/contents.json?verbose=false").parse()?;
    let resp = client.get(uri).await?;
    NODE_REST_COUNTER
        .with_label_values(&["mempool/contents", "json"])
        .inc();
    check_status(resp.status(), Error::RpcMempoolContent).await?;
    let body_bytes = hyper::body::to_bytes(resp.into_body()).await?;

    let content: FxHashSet<Txid> = if support_verbose {
        serde_json::from_reader(body_bytes.reader())?
    } else {
        let content: HashMap<Txid, Empty> = serde_json::from_reader(body_bytes.reader())?;
        content.into_keys().collect()
    };

    Ok(content)
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
