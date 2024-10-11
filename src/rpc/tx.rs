use std::str::FromStr;

use super::{check_status, CLIENT};
use crate::error::Error;
use crate::state::SerTx;
use crate::NODE_REST_COUNTER;
use bitcoin::consensus::serialize;
use bitcoin::{blockdata::constants::genesis_block, BlockHash, Network, Txid};
use hyper::body::Buf;
use once_cell::sync::Lazy;
use serde::Deserialize;

static GENESIS_TX: Lazy<Txid> = Lazy::new(|| {
    Txid::from_str("4a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b").unwrap()
});

// curl -s http://localhost:8332/rest/tx/3d0db8e24ffab61fb96e8a8fc5a0b14989b6e851495232018192b3e98f6b904e.json | jq
pub async fn call_json(txid: Txid) -> Result<TxJson, Error> {
    if txid == *GENESIS_TX {
        return Err(Error::GenesisTx);
    }
    let client = CLIENT.clone();
    let bitcoind_addr = crate::globals::bitcoind_addr();

    let uri = format!("http://{bitcoind_addr}/rest/tx/{txid}.json").parse()?;
    let resp = client.get(uri).await?;
    NODE_REST_COUNTER.with_label_values(&["tx", "json"]).inc();
    check_status(resp.status(), |s| Error::RpcTxJson(s, txid)).await?;
    let body_bytes = hyper::body::to_bytes(resp.into_body()).await?;
    let tx: TxJson = serde_json::from_reader(body_bytes.reader())?;
    Ok(tx)
}

pub async fn call_parse_json(
    txid: Txid,
    network: Network,
) -> Result<(Option<BlockHash>, SerTx), Error> {
    Ok(match call_json(txid).await {
        Ok(tx_json) => (tx_json.block_hash, SerTx(hex::decode(&tx_json.hex)?)),
        Err(Error::GenesisTx) => {
            let mut block = genesis_block(network);
            (
                Some(block.block_hash()),
                SerTx(serialize(&block.txdata.remove(0))),
            )
        }
        Err(e) => return Err(e),
    })
}

pub async fn call_raw(txid: Txid) -> Result<Vec<u8>, Error> {
    if txid == *GENESIS_TX {
        return Err(Error::GenesisTx);
    }
    let client = CLIENT.clone();
    let bitcoind_addr = crate::globals::bitcoind_addr();

    let uri = format!("http://{bitcoind_addr}/rest/tx/{txid}.bin").parse()?;
    let resp = client.get(uri).await?;
    NODE_REST_COUNTER.with_label_values(&["tx", "bin"]).inc();

    check_status(resp.status(), |s| Error::RpcTx(s, txid)).await?;
    let body_bytes = hyper::body::to_bytes(resp.into_body()).await?;
    Ok(body_bytes.to_vec())
}

#[derive(Deserialize, Debug, Clone)]
pub struct TxJson {
    // pub txid: Txid,
    // pub hash: String,
    // pub version: u32,
    // pub size: u32,
    // pub vsize: u32,
    // pub weight: u32,
    // pub locktime: u32,
    // pub vin: Vec<TxIn>,
    // pub vout: Vec<TxOut>,
    #[serde(rename = "blockhash")]
    pub block_hash: Option<BlockHash>,
    pub hex: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct TxIn {
    pub coinbase: Option<String>,
    pub txid: Option<String>,
    pub vout: Option<u32>,
    #[serde(rename = "scriptSig")]
    pub script_sig: Option<ScriptSig>,
    #[serde(default)]
    pub txinwitness: Vec<String>,
    pub sequence: u32,
}

#[derive(Deserialize, Debug, Clone)]
pub struct ScriptSig {
    pub asm: String,
    pub hex: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct TxOut {
    pub value: f64,
    pub n: u32,
    #[serde(rename = "scriptPubKey")]
    pub script_pubkey: ScriptPubKey,
}

#[derive(Deserialize, Debug, Clone)]
pub struct ScriptPubKey {
    pub asm: String,
    pub hex: String,
    pub address: Option<String>,
    pub r#type: String,
}
