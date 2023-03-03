// GET /rest/mempool/info.json
// GET /rest/mempool/contents.json

use super::{check_status, CLIENT};
use crate::error::Error;
use bitcoin::Txid;
use bitcoin_hashes::hex::FromHex;
use hyper::body::Buf;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

// curl -s http://localhost:8332/rest/mempool/info.json | jq
pub async fn info() -> Result<MempoolInfo, Error> {
    let client = CLIENT.clone();
    let bitcoind_addr = crate::globals::bitcoind_addr();

    let uri = format!("http://{bitcoind_addr}/rest/mempool/info.json").parse()?;
    let resp = client.get(uri).await?;
    check_status(resp.status(), Error::RpcMempoolInfo).await?;
    let body_bytes = hyper::body::to_bytes(resp.into_body()).await?;
    let info: MempoolInfo = serde_json::from_reader(body_bytes.reader())?;
    Ok(info)
}

// curl -s http://localhost:8332/rest/mempool/contents.json | jq
pub async fn content() -> Result<HashSet<Txid>, Error> {
    let client = CLIENT.clone();
    let bitcoind_addr = crate::globals::bitcoind_addr();

    let uri = format!("http://{bitcoind_addr}/rest/mempool/contents.json").parse()?;
    let resp = client.get(uri).await?;
    check_status(resp.status(), Error::RpcMempoolContent).await?;
    let body_bytes = hyper::body::to_bytes(resp.into_body()).await?;
    let content: HashMap<String, Value> = serde_json::from_reader(body_bytes.reader())?;
    let txids: HashSet<_> = content.keys().flat_map(|e| Txid::from_hex(e)).collect();
    Ok(txids)
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MempoolInfo {
    pub loaded: bool,
    pub size: u32,
    pub bytes: u32,
    pub usage: u32,
    pub total_fee: f64,
    pub maxmempool: u32,
    pub mempoolminfee: f64,
    pub minrelaytxfee: f64,
    pub unbroadcastcount: u32,
}
