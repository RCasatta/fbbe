use bitcoin::BlockHash;
use hyper::body::Buf;
use serde::Deserialize;
use tokio::time::sleep;

use crate::error::Error;

use super::CLIENT;

// curl -s http://localhost:8332/rest/chaininfo.json | jq
#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ChainInfo {
    pub chain: String,
    pub blocks: u32,
    #[serde(rename = "bestblockhash")]
    pub best_block_hash: BlockHash,
}

// curl -s http://localhost:8332/rest/chaininfo.json | jq

pub async fn call() -> Result<ChainInfo, Error> {
    let client = CLIENT.clone();
    let bitcoind_addr = crate::globals::bitcoind_addr();
    let uri = format!("http://{bitcoind_addr}/rest/chaininfo.json",).parse()?;
    let resp = client.get(uri).await?;
    if resp.status() != 200 {
        sleep(tokio::time::Duration::from_millis(10)).await;
        return Err(Error::RpcChainInfo);
    }
    let body_bytes = hyper::body::to_bytes(resp.into_body()).await?;
    let info: ChainInfo = serde_json::from_reader(body_bytes.reader())?;
    Ok(info)
}
