use std::{hash::Hasher, path::Path};

use bitcoin::{Block, Script};
use fxhash::FxHasher64;
use redb::{ReadableTable, RedbKey, RedbValue, TableDefinition};

use crate::{
    error::Error,
    rpc::{self, chaininfo::ChainInfo},
};

pub(crate) struct Database(redb::Database);

// the db is huge, like 30GB for 200k blocks, maybe try rocksdb

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

impl RedbValue for ScriptHashHeight {
    type SelfType<'a> = ScriptHashHeight where Self: 'a;

    type AsBytes<'a>  = &'a [u8] where Self: 'a;

    fn fixed_width() -> Option<usize> {
        Some(12)
    }

    fn from_bytes<'a>(data: &'a [u8]) -> Self::SelfType<'a>
    where
        Self: 'a,
    {
        ScriptHashHeight(data.try_into().unwrap())
    }

    fn as_bytes<'a, 'b: 'a>(value: &'a Self::SelfType<'b>) -> Self::AsBytes<'a>
    where
        Self: 'a,
        Self: 'b,
    {
        &value.0[..]
    }

    fn type_name() -> redb::TypeName {
        redb::TypeName::new("ScriptHashHeight")
    }
}
impl RedbKey for ScriptHashHeight {
    fn compare(data1: &[u8], data2: &[u8]) -> std::cmp::Ordering {
        data1.cmp(data2)
    }
}

const INITIAL_SYNC: TableDefinition<(), u32> = TableDefinition::new("initial_sync");

const ADDRESSES: TableDefinition<ScriptHashHeight, ()> = TableDefinition::new("address_height");

impl Database {
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self, redb::Error> {
        let db = redb::Database::create(path)?;

        let tables: Vec<_> = {
            let read_txn = db.begin_read()?;
            read_txn.list_tables()?.collect()
        };
        if tables.len() != 2 {
            // Creating DB
            let write_txn: redb::WriteTransaction<'_> = db.begin_write()?;
            write_txn.open_table(INITIAL_SYNC)?;
            write_txn.open_table(ADDRESSES)?;
            write_txn.commit()?;
        }

        Ok(Self(db))
    }

    fn last_synced_height(&self) -> Result<u32, redb::Error> {
        let read_txn = self.0.begin_read()?;
        let table = read_txn.open_table(INITIAL_SYNC)?;
        let height = table.get(())?.map(|a| a.value()).unwrap_or_default();
        Ok(height)
    }

    fn index_block(&self, block: &Block, height: u32) -> Result<(), redb::Error> {
        let mut write_txn: redb::WriteTransaction<'_> = self.0.begin_write()?;
        {
            if height % 10_000 != 0 {
                write_txn.set_durability(redb::Durability::None);
            }

            let mut addresses_table = write_txn.open_table(ADDRESSES)?;

            for tx in block.txdata.iter() {
                for output in tx.output.iter() {
                    if !output.script_pubkey.is_provably_unspendable() {
                        let key = ScriptHashHeight::new(&output.script_pubkey, height);
                        addresses_table.insert(key, ())?;
                    }
                }
            }
            let mut last_sync_height = write_txn.open_table(INITIAL_SYNC)?;
            last_sync_height.insert((), height)?;
        }
        write_txn.commit()?;

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
    let last_synced_height = db.last_synced_height()?;

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
