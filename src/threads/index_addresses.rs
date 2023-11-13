use std::{collections::HashMap, hash::Hasher, path::Path};

use bitcoin::{Block, OutPoint, Script};
use fxhash::FxHasher64;
use rocksdb::{WriteBatch, DB};

use crate::{
    error::Error,
    rpc::{self, chaininfo::ChainInfo},
};

#[derive(Debug)]
struct ScriptHashHeight([u8; 12]);

// TODO: move to 8 bytes key for script hash (initialized with xor to avoid attacks)
// and value equal to varint of every height delta in which the hash is found
// examples:
// 1) s found at h1 save varint(h1)
// 2) s found at h1 and h2 where h1<h2, save varint(h1) and varint(h2-h1)

fn script_hash(script: &Script) -> u64 {
    let mut hasher = FxHasher64::default();
    hasher.write(script.as_bytes());
    hasher.finish()
}
impl ScriptHashHeight {
    pub fn new(script_hash: u64, height: u32) -> Self {
        let mut data = [0u8; 12];
        data[..8].copy_from_slice(&script_hash.to_le_bytes()[..]);
        data[8..].copy_from_slice(&height.to_le_bytes()[..]);
        Self(data)
    }
}

impl AsRef<[u8]> for ScriptHashHeight {
    fn as_ref(&self) -> &[u8] {
        &self.0[..]
    }
}

pub struct Database(DB);

impl Database {
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self, rocksdb::Error> {
        let db = DB::open_default(path).unwrap();

        Ok(Self(db))
    }

    fn last_synced_height(&self) -> Result<Option<u32>, rocksdb::Error> {
        Ok(self
            .0
            .get(b"last_synced_height")?
            .map(|v| u32::from_le_bytes(v.try_into().unwrap())))
    }

    fn index_block(
        &self,
        block: &Block,
        height: u32,
        utxh: &mut HashMap<OutPoint, u64>,
    ) -> Result<(), rocksdb::Error> {
        // get hash if synced skip

        let mut batch = WriteBatch::default();

        {
            for tx in block.txdata.iter() {
                let txid = tx.txid();
                for (i, output) in tx.output.iter().enumerate() {
                    if !output.script_pubkey.is_provably_unspendable() {
                        let hash = script_hash(&output.script_pubkey);
                        utxh.insert(OutPoint::new(txid, i as u32), hash);
                        let key = ScriptHashHeight::new(hash, height);
                        batch.put(key, &[]);
                    }
                }
                if !tx.is_coin_base() {
                    for input in tx.input.iter() {
                        let hash = utxh.remove(&input.previous_output).unwrap();

                        let key = ScriptHashHeight::new(hash, height);
                        batch.put(key, &[]);
                    }
                }
            }

            // TODO, inputs? // save in temporary cache? ask core for previous output if missing?

            batch.put(b"last_synced_height", height.to_be_bytes()); // Switch to synced block hash?
        }
        self.0.write(batch)?;

        Ok(())
    }
}

pub(crate) async fn index_addresses_infallible(db: &Database, chain_info: ChainInfo) {
    if let Err(e) = index_addresses(db, chain_info).await {
        log::error!("{:?}", e);
    }
}

async fn index_addresses(db: &Database, chain_info: ChainInfo) -> Result<(), Error> {
    let last_synced_height = db.last_synced_height()?.unwrap_or(0);
    log::info!("Starting index_addresses from: {last_synced_height}");

    let mut utxh: HashMap<OutPoint, u64> = HashMap::new();

    for height in last_synced_height..chain_info.blocks {
        let hash = rpc::blockhashbyheight::call(height as usize).await?;
        let block = rpc::block::call_raw(hash.block_hash).await?;
        db.index_block(&block, height, &mut utxh)?;
        if height % 10_000 == 0 {
            log::info!("indexed block {height}")
        }
    }
    Ok(())
}

#[cfg(test)]
mod test {
    #[test]
    fn test_endianness() {
        let value = 1u64;
        assert_eq!(value.to_ne_bytes(), value.to_le_bytes());
    }
}
