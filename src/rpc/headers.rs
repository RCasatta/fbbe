// curl -s http://localhost:8332/rest/headers/1/000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f.json | jq

use std::{
    io::BufReader,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use super::{check_status, ts_to_date_time_utc, CLIENT};
use crate::{error::Error, NODE_REST_COUNTER};
use bitcoin::{consensus::Decodable, BlockHash};
use hyper::body::Buf;
use serde::Deserialize;

pub async fn call_many(
    block_hash: BlockHash,
    count: u32,
) -> Result<Vec<bitcoin::block::Header>, Error> {
    let client = CLIENT.clone();
    let bitcoind_addr = crate::globals::bitcoind_addr();
    //let uri = format!("http://{bitcoind_addr}/rest/headers/{block_hash}.bin?count={count}").parse()?;  // TODO move to this with bitcoind 0.24
    let uri = format!("http://{bitcoind_addr}/rest/headers/{count}/{block_hash}.bin").parse()?;
    let resp = client.get(uri).await?;
    NODE_REST_COUNTER
        .with_label_values(&["headers/x", "bin"])
        .inc();
    check_status(resp.status(), |s| {
        Error::RpcBlockHeaders(s, block_hash, count)
    })
    .await?;
    let body_bytes = hyper::body::to_bytes(resp.into_body()).await?;
    let mut reader = BufReader::new(body_bytes.reader());

    let mut headers: Vec<bitcoin::block::Header> = Vec::with_capacity(count as usize);
    for _ in 0..count {
        match Decodable::consensus_decode(&mut reader) {
            Ok(header) => headers.push(header),
            Err(_) => break,
        }
    }

    Ok(headers)
}

pub async fn call_one(block_hash: BlockHash) -> Result<BlockheaderJson, Error> {
    let client = CLIENT.clone();
    let bitcoind_addr = crate::globals::bitcoind_addr();
    let uri = format!("http://{bitcoind_addr}/rest/headers/1/{block_hash}.json").parse()?;
    let resp = client.get(uri).await?;
    NODE_REST_COUNTER
        .with_label_values(&["headers/1", "bin"])
        .inc();
    check_status(resp.status(), |s| Error::RpcBlockHeaderJson(s, block_hash)).await?;
    let body_bytes = hyper::body::to_bytes(resp.into_body()).await?;
    let mut blockheader: Vec<BlockheaderJson> = serde_json::from_reader(body_bytes.reader())?;

    if blockheader.is_empty() {
        Err(Error::HeaderNotFound(block_hash))
    } else {
        Ok(blockheader.remove(0))
    }
}

#[derive(Deserialize, Debug)]
#[allow(dead_code)]
pub struct BlockheaderJson {
    pub hash: String,

    #[serde(flatten)]
    pub height_time: HeightTime,
}
impl BlockheaderJson {
    pub(crate) fn height(&self) -> u32 {
        self.height_time.height
    }
}

#[derive(Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeightTime {
    pub height: u32,
    pub time: u32,
}

impl HeightTime {
    pub fn date_time_utc(&self) -> String {
        ts_to_date_time_utc(self.time)
    }

    pub(crate) fn since_now(&self) -> std::time::Duration {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
        Duration::from_secs(now.as_secs().saturating_sub(self.time as u64))
    }
}
