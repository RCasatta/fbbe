use std::{
    collections::{BTreeSet, HashMap, HashSet},
    fmt::Display,
    hash::Hasher,
    path::Path,
    sync::Arc,
};

use bitcoin::{consensus::Decodable, Block, BlockHash, OutPoint, Script, Transaction, Txid};
use fxhash::FxHasher64;
use rocksdb::{ColumnFamily, ColumnFamilyDescriptor, Options, WriteBatch, DB};

use crate::{
    error::Error,
    rpc::{self, chaininfo::ChainInfo},
    state::SharedState,
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

    async fn index_block(
        &self,
        block: &Block,
        height: u32,
        shared_state: Arc<SharedState>,
    ) -> Result<HitRate, crate::Error> {
        let block_hash = block.block_hash();
        let mut hit_rate = HitRate::default();
        if self.is_block_hash_indexed(&block_hash) {
            hit_rate.already_indexed = 1;
            return Ok(hit_rate);
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
            let cached_tx = {
                let cached_txs = shared_state.txs.lock().await;
                cached_txs
                    .get(&txid)
                    .map(|mut sertx| Transaction::consensus_decode(&mut sertx))
                    .transpose()?
            };

            if cached_tx.is_some() {
                hit_rate.hit += 1;
            } else {
                hit_rate.miss += 1;
            }

            let tx = match cached_tx {
                Some(tx) => tx,
                None => rpc::tx::call_raw(txid).await?,
            };
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

        shared_state.update_cache(block, None).await?;

        let mut batch = WriteBatch::default();
        let height_bytes = height.to_le_bytes();

        let mut buffer = vec![];
        for script_hash in block_script_hashes {
            buffer.clear();
            buffer.extend(script_hash.to_le_bytes());
            buffer.extend(&height_bytes[..]);
            batch.put_cf(self.script_hash_cf(), &buffer, &[]);
        }
        batch.put_cf(self.block_hash_cf(), block_hash, &[]);

        self.db.write(batch)?;

        Ok(hit_rate)
    }
}

pub(crate) async fn index_addresses_infallible(
    db: &Database,
    chain_info: ChainInfo,
    shared_state: Arc<SharedState>,
) {
    if let Err(e) = index_addresses(db, chain_info, shared_state).await {
        log::error!("{:?}", e);
    }
}

#[derive(Default)]
struct HitRate {
    pub hit: u64,
    pub miss: u64,
    pub already_indexed: u64,
}

impl HitRate {
    pub fn rate(&self) -> f64 {
        (self.hit as f64) / (self.hit + self.miss) as f64
    }
}

impl std::ops::Add<HitRate> for HitRate {
    type Output = HitRate;

    fn add(self, rhs: HitRate) -> Self::Output {
        HitRate {
            hit: self.hit + rhs.hit,
            miss: self.miss + rhs.miss,
            already_indexed: self.already_indexed + rhs.already_indexed,
        }
    }
}

impl Display for HitRate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "already_indexed:{} hit:{} miss:{} rate:{:.2}",
            self.already_indexed,
            self.hit,
            self.miss,
            self.rate()
        )
    }
}

async fn index_addresses(
    db: &Database,
    chain_info: ChainInfo,
    shared_state: Arc<SharedState>,
) -> Result<(), Error> {
    log::info!("Starting index_addresses");

    let mut total_hit_rate = HitRate::default();

    for height in 0..chain_info.blocks {
        let hash = rpc::blockhashbyheight::call(height as usize).await?;
        let block = rpc::block::call_raw(hash.block_hash).await?;
        let hr = db.index_block(&block, height, shared_state.clone()).await?;
        total_hit_rate = total_hit_rate + hr;
        if height % 10_000 == 0 {
            log::info!("indexed block {height} {}", total_hit_rate)
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
