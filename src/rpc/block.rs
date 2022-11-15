// curl -s http://localhost:8332/rest/block/notxdetails/000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f.json | jq

use bitcoin::{consensus::deserialize, Block, BlockHash, Txid};
use hyper::body::Buf;
use maud::{html, Markup};
use serde::Deserialize;
use tokio::time::sleep;

use crate::{error::Error, globals::network, pages::NBSP, NetworkExt};

use super::{ts_to_date_time_utc, CLIENT};

pub async fn call_json(block_hash: BlockHash) -> Result<BlockNoTxDetails, Error> {
    let client = CLIENT.clone();
    let bitcoind_addr = crate::globals::bitcoind_addr();

    let uri =
        format!("http://{bitcoind_addr}/rest/block/notxdetails/{block_hash}.json",).parse()?;
    log::trace!("asking {:?}", uri);
    let resp = client.get(uri).await?;
    if resp.status() != 200 {
        sleep(tokio::time::Duration::from_millis(10)).await;
        return Err(Error::RpcBlockJson(block_hash));
    }
    let body_bytes = hyper::body::to_bytes(resp.into_body()).await?;
    let block: BlockNoTxDetails = serde_json::from_reader(body_bytes.reader())?;
    Ok(block)
}

pub async fn call_raw(block_hash: BlockHash) -> Result<Block, Error> {
    let client = CLIENT.clone();
    let bitcoind_addr = crate::globals::bitcoind_addr();

    let uri = format!("http://{bitcoind_addr}/rest/block/{block_hash}.bin",).parse()?;
    let resp = client.get(uri).await?;
    if resp.status() != 200 {
        sleep(tokio::time::Duration::from_millis(10)).await;
        return Err(Error::RpcBlockRaw);
    }
    let body_bytes = hyper::body::to_bytes(resp.into_body()).await?;
    let block: Block = deserialize(&body_bytes.to_vec())?;
    Ok(block)
}

#[derive(Deserialize)]
pub struct BlockNoTxDetails {
    pub hash: BlockHash,
    pub tx: Vec<Txid>,
    pub height: u32,
    pub version: u32,
    #[serde(rename = "versionHex")]
    pub version_hex: String,

    pub merkleroot: String,
    pub time: u32,
    pub previousblockhash: Option<String>,
    pub nextblockhash: Option<String>,
    pub size: usize,
    pub weight: usize,
    pub bits: String,
    pub difficulty: f64,

    pub nonce: u32,
}

impl BlockNoTxDetails {
    pub fn previous_block_hash_link(&self) -> Markup {
        match self.previousblockhash.as_ref() {
            Some(val) => {
                let link = format!("{}b/{}", network().as_url_path(), val);
                html! { a href=(link) { "«" } (NBSP) }
            }
            None => html! {},
        }
    }

    pub fn next_block_hash_link(&self) -> Markup {
        match self.nextblockhash.as_ref() {
            Some(val) => {
                let link = format!("{}b/{}", network().as_url_path(), val);
                html! { (NBSP) a href=(link) { "»" } }
            }
            None => html! {},
        }
    }

    pub fn date_time_utc(&self) -> String {
        ts_to_date_time_utc(self.time)
    }
}
