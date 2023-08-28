use std::{hash::Hasher, path::Path};

use bitcoin::{Block, Script};
use fxhash::FxHasher64;
use rocksdb::{WriteBatch, DB};

use crate::{
    error::Error,
    rpc::{self, chaininfo::ChainInfo},
};

#[derive(Debug)]
struct ScriptHashHeight([u8; 12]);

impl ScriptHashHeight {
    pub fn new(script: &Script, height: u32) -> Self {
        let mut hasher = FxHasher64::default();
        hasher.write(script.as_bytes());
        let hash = hasher.finish();
        let mut data = [0u8; 12];
        data[..8].copy_from_slice(&hash.to_le_bytes()[..]);
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

    fn index_block(&self, block: &Block, height: u32) -> Result<(), rocksdb::Error> {
        let mut batch = WriteBatch::default();

        {
            if height % 10_000 != 0 {
                log::info!("indexing block {height}");
            }

            for tx in block.txdata.iter() {
                for output in tx.output.iter() {
                    if !output.script_pubkey.is_provably_unspendable() {
                        let key = ScriptHashHeight::new(&output.script_pubkey, height);
                        batch.put(key, &[]);
                    }
                }
            }
            batch.put(b"last_synced_height", height.to_be_bytes());
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
    log::info!("Starting index_addresses");
    let last_synced_height = db.last_synced_height()?.unwrap_or(0);

    for height in last_synced_height..chain_info.blocks {
        let hash = rpc::blockhashbyheight::call(height as usize).await?;
        let block = rpc::block::call_raw(hash.block_hash).await?;
        db.index_block(&block, height)?;
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
