// curl -s http://localhost:8332/rest/block/notxdetails/000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f.json | jq

use super::{ts_to_date_time_utc, CLIENT};
use crate::{
    error::Error, globals::network, pages::NBSP, rpc::check_status, NetworkExt, NODE_REST_COUNTER,
};
use bitcoin::{consensus::deserialize, Block, BlockHash, Txid};
use hyper::body::Buf;
use maud::{html, Markup};
use serde::Deserialize;

pub struct SerBlock(pub Vec<u8>);

pub async fn call_json(block_hash: BlockHash) -> Result<BlockNoTxDetails, Error> {
    let client = CLIENT.clone();
    let bitcoind_addr = crate::globals::bitcoind_addr();

    let uri =
        format!("http://{bitcoind_addr}/rest/block/notxdetails/{block_hash}.json",).parse()?;
    log::trace!("asking {:?}", uri);
    let resp = client.get(uri).await?;
    NODE_REST_COUNTER
        .with_label_values(&["block/notxdetails", "json"])
        .inc();
    check_status(resp.status(), |s| Error::RpcBlockJson(s, block_hash)).await?;
    let body_bytes = http_body_util::BodyExt::collect(resp.into_body()).await?.to_bytes();
    let block: BlockNoTxDetails = serde_json::from_reader(body_bytes.reader())?;
    Ok(block)
}

pub async fn call(block_hash: BlockHash) -> Result<Block, Error> {
    let ser_block = call_raw(block_hash).await?;
    let block: Block = deserialize(&ser_block.0)?;
    Ok(block)
}

pub async fn call_raw(block_hash: BlockHash) -> Result<SerBlock, Error> {
    let client = CLIENT.clone();
    let bitcoind_addr = crate::globals::bitcoind_addr();

    let uri = format!("http://{bitcoind_addr}/rest/block/{block_hash}.bin",).parse()?;
    let resp = client.get(uri).await?;
    NODE_REST_COUNTER.with_label_values(&["block", "bin"]).inc();
    check_status(resp.status(), |s| Error::RpcBlockRaw(s, block_hash)).await?;
    let body_bytes = http_body_util::BodyExt::collect(resp.into_body()).await?.to_bytes();

    Ok(SerBlock(body_bytes.to_vec()))
}

#[derive(Deserialize)]
pub struct BlockNoTxDetails {
    pub hash: BlockHash,
    pub tx: Vec<Txid>,
    pub height: u32,
    #[allow(dead_code)]
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
