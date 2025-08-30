use super::{check_status, tx::ScriptPubKey, CLIENT};
use crate::{error::Error, NODE_REST_COUNTER};
use bitcoin::{BlockHash, Txid};
use hyper::body::Buf;
use serde::Deserialize;

// curl -s localhost:8332/rest/getutxos/checkmempool/f63db148598c3f3a7ae4590a7f70f16968e01872455281a8e487f6992721febc-0.json | jq
pub async fn _call(txid: Txid, vout: u32) -> Result<TxOutJson, Error> {
    let client = CLIENT.clone();
    let bitcoind_addr = crate::globals::bitcoind_addr();

    let uri =
        format!("http://{bitcoind_addr}/rest/getutxos/checkmempool/{txid}-{vout}.json").parse()?;
    let resp = client.get(uri).await?;
    NODE_REST_COUNTER
        .with_label_values(&["getutxos/checkmempool", "json"])
        .inc();

    check_status(resp.status(), |s| Error::RpcTxOut(s, txid, vout)).await?;
    let body_bytes = http_body_util::BodyExt::collect(resp.into_body()).await?.to_bytes();
    let tx: TxOutJson = serde_json::from_reader(body_bytes.reader())?;
    Ok(tx)
}

#[derive(Deserialize, Debug, Clone)]
#[allow(dead_code)]
pub struct TxOutJson {
    #[serde(rename = "chainHeight")]
    pub chain_height: u32,

    #[serde(rename = "chaintipHash")]
    pub chaintip_hash: BlockHash,

    pub bitmap: String,
    pub utxos: Vec<Utxo>,
}

#[derive(Deserialize, Debug, Clone)]
#[allow(dead_code)]
pub struct Utxo {
    pub height: u32,
    pub value: f64,

    #[serde(rename = "scriptPubKey")]
    pub script_pubkey: ScriptPubKey,
}
