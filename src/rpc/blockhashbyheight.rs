// curl -s localhost:8332/rest/blockhashbyheight/1.json

use super::{check_status, CLIENT};
use crate::{error::Error, NODE_REST_COUNTER};
use bitcoin::BlockHash;
use hyper::body::Buf;
use serde::Deserialize;

pub async fn _call(height: usize) -> Result<BlockHashByHeight, Error> {
    let client = CLIENT.clone();
    let bitcoind_addr = crate::globals::bitcoind_addr();
    let uri = format!("http://{bitcoind_addr}/rest/blockhashbyheight/{height}.json",).parse()?;
    let resp = client.get(uri).await?;
    NODE_REST_COUNTER
        .with_label_values(&["blockhashbyheight", "json"])
        .inc();

    check_status(resp.status(), |s| {
        Error::RpcBlockHashByHeightJson(s, height)
    })
    .await?;
    let body_bytes = hyper::body::to_bytes(resp.into_body()).await?;
    let hash: BlockHashByHeight = serde_json::from_reader(body_bytes.reader())?;
    Ok(hash)
}

#[derive(Deserialize, Debug)]
pub struct BlockHashByHeight {
    #[serde(rename = "blockhash")]
    pub block_hash: BlockHash,
}
