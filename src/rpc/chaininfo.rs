use super::{check_status, CLIENT};
use crate::error::Error;
use bitcoin::BlockHash;
use hyper::body::Buf;
use serde::Deserialize;

// curl -s http://localhost:8332/rest/chaininfo.json | jq
#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ChainInfo {
    pub chain: String,
    pub blocks: u32,
    #[serde(rename = "bestblockhash")]
    pub best_block_hash: BlockHash,
    #[serde(rename = "initialblockdownload")]
    pub initial_block_download: bool,

    pub size_on_disk: u64,
}

// curl -s http://localhost:8332/rest/chaininfo.json | jq

pub async fn call() -> Result<ChainInfo, Error> {
    let client = CLIENT.clone();
    let bitcoind_addr = crate::globals::bitcoind_addr();
    let uri = format!("http://{bitcoind_addr}/rest/chaininfo.json",).parse()?;
    let resp = client.get(uri).await?;
    check_status(resp.status(), Error::RpcChainInfo).await?;
    let body_bytes = hyper::body::to_bytes(resp.into_body()).await?;
    let info: ChainInfo = serde_json::from_reader(body_bytes.reader())?;
    Ok(info)
}
