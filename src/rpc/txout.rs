use super::tx::ScriptPubKey;
use super::CLIENT;
use crate::error::Error;
use bitcoin::{BlockHash, Txid};
use hyper::body::Buf;
use serde::Deserialize;
use tokio::time::sleep;

// curl -s localhost:8332/rest/getutxos/checkmempool/f63db148598c3f3a7ae4590a7f70f16968e01872455281a8e487f6992721febc-0.json | jq
pub async fn call(txid: &Txid, vout: u32) -> Result<TxOutJson, Error> {
    let client = CLIENT.clone();
    let bitcoind_addr = crate::globals::bitcoind_addr();

    let uri =
        format!("http://{bitcoind_addr}/rest/getutxos/checkmempool/{txid}-{vout}.json").parse()?;
    let resp = client.get(uri).await?;
    if resp.status() != 200 {
        sleep(tokio::time::Duration::from_millis(10)).await;
        return Err(Error::RpcTxOut);
    }
    let body_bytes = hyper::body::to_bytes(resp.into_body()).await?;
    let tx: TxOutJson = serde_json::from_reader(body_bytes.reader())?;
    Ok(tx)
}

#[derive(Deserialize, Debug, Clone)]
pub struct TxOutJson {
    #[serde(rename = "chainHeight")]
    pub chain_height: u32,

    #[serde(rename = "chaintipHash")]
    pub chaintip_hash: BlockHash,

    pub bitmap: String,
    pub utxos: Vec<Utxo>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Utxo {
    pub height: u32,
    pub value: f64,

    #[serde(rename = "scriptPubKey")]
    pub script_pubkey: ScriptPubKey,
}
