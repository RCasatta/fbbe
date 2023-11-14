use std::{
    collections::{BTreeSet, HashMap, HashSet},
    hash::Hasher,
    path::Path,
};

use bitcoin::{Block, BlockHash, OutPoint, Script, Transaction, Txid};
use fxhash::FxHasher64;
use rocksdb::{ColumnFamily, ColumnFamilyDescriptor, Options, WriteBatch, DB};

use crate::{
    error::Error,
    rpc::{self, chaininfo::ChainInfo},
};

#[derive(Debug)]
struct ScriptHashHeight([u8; 12]);

#[derive(Eq, Hash, PartialEq)]
struct TruncOutPoint(u128);

impl From<&OutPoint> for TruncOutPoint {
    fn from(value: &OutPoint) -> Self {
        let mut v = u128::from_le_bytes((&value.txid[..16]).try_into().unwrap());
        v += value.vout as u128;

        TruncOutPoint(v)
    }
}

impl From<OutPoint> for TruncOutPoint {
    fn from(value: OutPoint) -> Self {
        From::from(&value)
    }
}

// TODO: move to 8 bytes key for script hash (initialized with xor to avoid attacks)
// and value equal to varint of every height delta in which the hash is found
// examples:
// 1) s found at h1 save varint(h1)
// 2) s found at h1 and h2 where h1<h2, save varint(h1) and varint(h2-h1)

type ScriptHash = u64;
fn script_hash(script: &Script) -> ScriptHash {
    let mut hasher = FxHasher64::default();
    hasher.write(script.as_bytes());
    hasher.finish()
}

impl AsRef<[u8]> for ScriptHashHeight {
    fn as_ref(&self) -> &[u8] {
        &self.0[..]
    }
}

const BLOCK_HASH_CF: &str = "BLOCK_HASH_CF";
const SCRIPT_HASH_CF: &str = "SCRIPT_HASH_CF";

const COLUMN_FAMILIES: &[&str] = &[BLOCK_HASH_CF, SCRIPT_HASH_CF];

pub struct Database {
    db: DB,
}

impl Database {
    fn create_cf_descriptors() -> Vec<ColumnFamilyDescriptor> {
        COLUMN_FAMILIES
            .iter()
            .map(|&name| ColumnFamilyDescriptor::new(name, Options::default()))
            .collect()
    }

    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self, rocksdb::Error> {
        let mut db_opts = Options::default();

        db_opts.create_if_missing(true);
        db_opts.create_missing_column_families(true);

        let db = DB::open_cf_descriptors(&db_opts, path, Self::create_cf_descriptors())?;
        Ok(Self { db })
    }

    fn block_hash_cf(&self) -> &ColumnFamily {
        self.db
            .cf_handle(BLOCK_HASH_CF)
            .expect("missing BLOCK_HASH_CF")
    }

    fn is_block_hash_indexed(&self, block_hash: &BlockHash) -> bool {
        self.db
            .get_pinned_cf(self.block_hash_cf(), block_hash)
            .unwrap()
            .is_some()
    }

    fn script_hash_cf(&self) -> &ColumnFamily {
        self.db
            .cf_handle("SCRIPT_HASH_CF")
            .expect("missing SCRIPT_HASH_CF")
    }

    async fn index_block(&self, block: &Block, height: u32) -> Result<(), crate::Error> {
        let block_hash = block.block_hash();
        if self.is_block_hash_indexed(&block_hash) {
            return Ok(());
        }

        // ## script_pubkeys in outputs, easy
        let mut block_script_hashes: BTreeSet<ScriptHash> = block
            .txdata
            .iter()
            .flat_map(|tx| tx.output.iter())
            .map(|txout| script_hash(&txout.script_pubkey))
            .collect();

        // ## script_pubkeys in previouts outputs

        // ### we don't consider outputs created in the same block
        let mut outputs_in_block: HashSet<OutPoint> = HashSet::new();
        for tx in block.txdata.iter() {
            let txid = tx.txid();
            for i in 0..tx.output.len() {
                outputs_in_block.insert(OutPoint::new(txid, i as u32));
            }
        }
        let prevouts_in_block: HashSet<OutPoint> = block
            .txdata
            .iter()
            .filter(|tx| !tx.is_coin_base())
            .flat_map(|tx| tx.input.iter())
            .map(|e| e.previous_output)
            .collect();
        let txid_needed: HashSet<Txid> = prevouts_in_block
            .difference(&outputs_in_block)
            .map(|o| o.txid)
            .collect();

        // ### getting all transactions for prevouts
        let mut transactions: HashMap<Txid, Transaction> = HashMap::new();
        for txid in txid_needed {
            let tx = rpc::tx::call_raw(txid).await?;
            transactions.insert(txid, tx);
        }

        for tx in block.txdata.iter() {
            if tx.is_coin_base() {
                continue;
            }

            for input in tx.input.iter() {
                if outputs_in_block.contains(&input.previous_output) {
                    // script already considered with the output iteration
                    continue;
                }
                let tx = transactions.get(&input.previous_output.txid).unwrap(); // all previous transactions have been fetched
                let prevout = &tx.output[input.previous_output.vout as usize];
                block_script_hashes.insert(script_hash(&prevout.script_pubkey));
            }
        }

        let mut batch = WriteBatch::default();
        let height_bytes = height.to_le_bytes();

        for script_hash in block_script_hashes {
            batch.put_cf(
                self.script_hash_cf(),
                &script_hash.to_le_bytes(),
                &height_bytes[..],
            );
        }
        batch.put_cf(self.block_hash_cf(), block_hash, &[]);

        self.db.write(batch)?;

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

    for height in 0..chain_info.blocks {
        let hash = rpc::blockhashbyheight::call(height as usize).await?;
        let block = rpc::block::call_raw(hash.block_hash).await?;
        db.index_block(&block, height).await?;
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
