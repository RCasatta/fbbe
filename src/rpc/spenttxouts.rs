// curl -s http://localhost:8332/rest/spenttxouts/000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f.hex

use super::{check_status, CLIENT};
use crate::{error::Error, NODE_REST_COUNTER};
use bitcoin::{
    consensus::{encode, Decodable},
    BlockHash, TxOut, VarInt,
};

/// The outputs spent by each transaction of a block, in block order, the coinbase spends none.
/// Requires Bitcoin Core 30 or later.
pub async fn call(block_hash: BlockHash) -> Result<Vec<Vec<TxOut>>, Error> {
    let client = CLIENT.clone();
    let bitcoind_addr = crate::globals::bitcoind_addr();

    let uri = format!("http://{bitcoind_addr}/rest/spenttxouts/{block_hash}.bin").parse()?;
    let resp = client.get(uri).await?;
    NODE_REST_COUNTER
        .with_label_values(&["spenttxouts", "bin", resp.status().as_str()])
        .inc();
    check_status(resp.status(), |s| Error::RpcSpentTxOuts(s, block_hash)).await?;
    let body_bytes = http_body_util::BodyExt::collect(resp.into_body())
        .await?
        .to_bytes();

    Ok(decode(&body_bytes)?)
}

/// Decodes the binary format: a compact size count of transactions, then the consensus encoding
/// of the outputs each one spends.
pub fn decode(mut bytes: &[u8]) -> Result<Vec<Vec<TxOut>>, encode::Error> {
    let txs = VarInt::consensus_decode(&mut bytes)?.0;
    // the count comes from the node, still it doesn't size the allocation upfront
    let mut result = Vec::new();
    for _ in 0..txs {
        result.push(Vec::<TxOut>::consensus_decode(&mut bytes)?);
    }
    if !bytes.is_empty() {
        return Err(encode::Error::ParseFailed(
            "trailing bytes in spent outputs",
        ));
    }
    Ok(result)
}

#[cfg(test)]
mod test {
    #[test]
    fn decode_spent_outputs() {
        // regtest block with a coinbase and one tx spending a 50 BTC p2wpkh output
        let bytes =
            hex::decode("02000100f2052a01000000160014c2630d6981656fbc97448c5cbaf7531586f5fe98")
                .unwrap();
        let spent = super::decode(&bytes).unwrap();
        assert_eq!(spent.len(), 2);
        assert!(spent[0].is_empty());
        assert_eq!(spent[1].len(), 1);
        assert_eq!(spent[1][0].value.to_sat(), 5_000_000_000);
        assert_eq!(
            spent[1][0].script_pubkey.as_bytes(),
            hex::decode("0014c2630d6981656fbc97448c5cbaf7531586f5fe98").unwrap()
        );

        let genesis = hex::decode("0100").unwrap();
        assert_eq!(super::decode(&genesis).unwrap(), vec![vec![]]);

        assert!(super::decode(&bytes[..bytes.len() - 1]).is_err());
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(super::decode(&trailing).is_err());
    }
}
