use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Display,
    hash::Hasher,
    path::Path,
    sync::Arc,
};

use bitcoin::{Block, BlockHash, OutPoint, Script};
use fxhash::FxHasher64;
use rocksdb::{ColumnFamily, ColumnFamilyDescriptor, Options, WriteBatch, DB};

use crate::{
    error::Error,
    rpc::{self, chaininfo::ChainInfo},
    state::SharedState,
};

#[derive(Debug)]
struct ScriptHashHeight([u8; 12]);

type ScriptHash = u64;
type Height = u32;

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
const FUNDING_CF: &str = "FUNDING_CF";
const SPENDING_CF: &str = "SPENDING_CF";

const COLUMN_FAMILIES: &[&str] = &[BLOCK_HASH_CF, FUNDING_CF, SPENDING_CF];

#[derive(Debug)]
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

    fn funding_cf(&self) -> &ColumnFamily {
        self.db.cf_handle("FUNDING_CF").expect("missing FUNDING_CF")
    }

    fn spending_cf(&self) -> &ColumnFamily {
        self.db
            .cf_handle("SPENDING_CF")
            .expect("missing SPENDING_CF")
    }

    pub fn script_hash_heights(&self, script_pubkey: &Script) -> Vec<u32> {
        let script_hash = script_hash(script_pubkey).to_le_bytes();
        let mut result = vec![];

        for el in self.db.iterator_cf(
            self.funding_cf(),
            rocksdb::IteratorMode::From(&script_hash[..], rocksdb::Direction::Forward),
        ) {
            let el = el.unwrap().0;
            if el.starts_with(&script_hash) {
                result.push(u32::from_le_bytes(el[8..].try_into().unwrap()));
            } else {
                break;
            }
        }

        result
    }

    pub fn write_hashes(&self, index_res: IndexBlockResult) {
        // TODO move following code outside, return block_script_hashes and block hash so we don't depend on self

        let mut batch = WriteBatch::default();
        let height_bytes = index_res.height.to_le_bytes();

        let mut buffer = vec![];
        for script_hash in index_res.funding_sh {
            buffer.clear();
            buffer.extend(script_hash.to_le_bytes());
            buffer.extend(&height_bytes[..]);
            batch.put_cf(self.funding_cf(), &buffer, &[]);
        }
        for (out_point, height) in index_res.spending_sh {
            buffer.clear();
            let mut val = u64::from_le_bytes((&out_point.txid[..8]).try_into().unwrap());
            val += out_point.vout as u64;
            buffer.extend(val.to_le_bytes());
            buffer.extend(&height.to_le_bytes()[..]);
            batch.put_cf(self.spending_cf(), &buffer, &[]);
        }

        batch.put_cf(self.block_hash_cf(), index_res.block_hash, &[]);

        self.db.write(batch).unwrap();
    }
}

pub struct IndexBlockResult {
    block_hash: BlockHash,
    height: Height,
    txid_blockhash_hit_rate: HitRate,

    funding_sh: BTreeSet<ScriptHash>,
    spending_sh: BTreeMap<OutPoint, Height>,
}
async fn index_block(
    block: &Block,
    height: u32,
    shared_state: Arc<SharedState>,
) -> Result<IndexBlockResult, crate::Error> {
    let block_hash = block.block_hash();
    let mut txid_blockhash_hit_rate = HitRate::default();

    shared_state.update_cache(block, Some(height)).await?;

    // # funding script_hashes, script_pubkeys in outputs
    let funding_sh: BTreeSet<ScriptHash> = block
        .txdata
        .iter()
        .flat_map(|tx| tx.output.iter())
        .map(|txout| script_hash(&txout.script_pubkey))
        .collect();

    // # spending script_hashes
    let mut spending_sh: BTreeMap<OutPoint, u32> = BTreeMap::new();
    for tx in block.txdata.iter() {
        if tx.is_coin_base() {
            continue;
        }

        for input in tx.input.iter() {
            let outpoint = input.previous_output;
            let txid = input.previous_output.txid;
            let block_hash = match shared_state.tx_in_block.lock().await.get(&txid) {
                Some(block_hash) => {
                    txid_blockhash_hit_rate.hit += 1;
                    *block_hash
                }
                None => {
                    txid_blockhash_hit_rate.miss += 1;
                    rpc::tx::call_json(txid).await?.block_hash.unwrap()
                }
            };
            let height = match shared_state
                .hash_to_height_time
                .lock()
                .await
                .get(&block_hash)
            {
                Some(ht) => ht.height,
                None => {
                    log::error!("should never happen I haven't seen a prevout txid block height");
                    rpc::block::call_json(block_hash).await?.height
                }
            };

            spending_sh.insert(outpoint, height);
        }
    }

    Ok(IndexBlockResult {
        block_hash,
        height,
        txid_blockhash_hit_rate,
        funding_sh,
        spending_sh,
    })
}

pub(crate) async fn index_addresses_infallible(
    db: Arc<Database>,
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
}

impl HitRate {
    pub fn rate(&self) -> f64 {
        (self.hit as f64) / (self.hit + self.miss) as f64
    }
}

impl std::ops::Add<&HitRate> for HitRate {
    type Output = HitRate;

    fn add(self, rhs: &HitRate) -> Self::Output {
        HitRate {
            hit: self.hit + rhs.hit,
            miss: self.miss + rhs.miss,
        }
    }
}

impl Display for HitRate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "hit:{} miss:{} rate:{:.2}",
            self.hit,
            self.miss,
            self.rate()
        )
    }
}

async fn index_addresses(
    db: Arc<Database>,
    chain_info: ChainInfo,
    shared_state: Arc<SharedState>,
) -> Result<(), Error> {
    log::info!("Starting index_addresses");

    let mut txid_blockhash_total_hit_rate = HitRate::default();

    let mut already_indexed = 0;
    for height in 0..chain_info.blocks {
        let hash = rpc::blockhashbyheight::call(height as usize).await?;
        let block = rpc::block::call_raw(hash.block_hash).await?;
        let block_hash = block.block_hash();
        if db.is_block_hash_indexed(&block_hash) {
            already_indexed += 1;
        } else {
            let index_res = index_block(&block, height, shared_state.clone()).await?;
            txid_blockhash_total_hit_rate =
                txid_blockhash_total_hit_rate + &index_res.txid_blockhash_hit_rate;
            let db = db.clone();
            tokio::spawn(async move { db.write_hashes(index_res) });
        }
        if height % 10_000 == 0 {
            log::info!(
                "indexed block {height} txid_bh({txid_blockhash_total_hit_rate}) already_indexed:{already_indexed}"
            )
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
