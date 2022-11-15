// curl -s localhost:8332/rest/blockhashbyheight/1.json

use bitcoin::BlockHash;
use hyper::body::Buf;
use serde::Deserialize;
use tokio::time::sleep;

use crate::error::Error;

use super::CLIENT;

pub async fn call(height: usize) -> Result<BlockHashByHeight, Error> {
    let client = CLIENT.clone();
    let bitcoind_addr = crate::globals::bitcoind_addr();
    let uri = format!("http://{bitcoind_addr}/rest/blockhashbyheight/{height}.json",).parse()?;
    let resp = client.get(uri).await?;
    if resp.status() != 200 {
        sleep(tokio::time::Duration::from_millis(10)).await;
        return Err(Error::RpcBlockHashByHeightJson);
    }
    let body_bytes = hyper::body::to_bytes(resp.into_body()).await?;
    let hash: BlockHashByHeight = serde_json::from_reader(body_bytes.reader())?;
    Ok(hash)
}

#[derive(Deserialize, Debug)]
pub struct BlockHashByHeight {
    #[serde(rename = "blockhash")]
    pub block_hash: BlockHash,
}
