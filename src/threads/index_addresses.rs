use std::{
    collections::BTreeSet, fmt::Display, hash::Hasher, ops::ControlFlow, path::Path, sync::Arc,
};

use bitcoin::{hashes::Hash, Address, Block, BlockHash, OutPoint, Script, ScriptBuf, Txid};
use bitcoin_slices::{bsl, Visit, Visitor};
use fxhash::FxHasher64;
use rocksdb::{ColumnFamily, ColumnFamilyDescriptor, Options, WriteBatch, DB};

use crate::{
    error::Error,
    rpc::{self, block::SerBlock, chaininfo::ChainInfo},
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

    pub fn outputs_heights(&self, txids: &[Txid]) -> Vec<u32> {
        // iter on spending_cf, return all results found up to gap
        todo!()
    }

    pub fn write_hashes(&self, index_res: IndexBlockResult) {
        let mut batch = WriteBatch::default();
        let height_bytes = index_res.height.to_le_bytes();

        let mut buffer = vec![];
        for script_hash in index_res.funding_sh {
            buffer.clear();
            buffer.extend(script_hash.to_le_bytes());
            buffer.extend(&height_bytes[..]);
            batch.put_cf(self.funding_cf(), &buffer, &[]);
        }
        for out_point in index_res.spending_sh {
            buffer.clear();
            let mut val = u64::from_le_bytes((&out_point.txid[..8]).try_into().unwrap());
            val += out_point.vout as u64;
            buffer.extend(val.to_le_bytes());
            buffer.extend(&height_bytes[..]);
            batch.put_cf(self.spending_cf(), &buffer, &[]);
        }

        batch.put_cf(self.block_hash_cf(), index_res.block_hash, &[]);

        self.db.write(batch).unwrap();
    }
}

pub struct IndexBlockResult {
    block_hash: BlockHash,
    height: Height,

    funding_sh: BTreeSet<ScriptHash>,
    spending_sh: BTreeSet<OutPoint>,
}

pub async fn txids_with_address(
    address: &Address,
    db: Arc<Database>,
    shared_state: Arc<SharedState>,
) -> Result<Vec<Txid>, Error> {
    let script_pubkey = address.script_pubkey();
    let heights = db.script_hash_heights(&script_pubkey);
    let blocks = shared_state.blocks_from_heights(&heights).await?;
    let mut txids = vec![];
    for (_, b) in blocks {
        find_txs_with_script_pubkey(&script_pubkey, b, &mut txids);
    }

    let heights = db.outputs_heights(&txids);
    let blocks = shared_state.blocks_from_heights(&heights).await?;

    for (_, b) in blocks {
        find_txs_with_prevout(b, &mut txids);
    }

    todo!()
}

fn find_txs_with_prevout(b: SerBlock, txids: &mut Vec<Txid>) {
    struct TxContainingOutpoint<'a> {
        txids: &'a mut Vec<Txid>,
        found: bool,
    }

    impl<'a> Visitor for TxContainingOutpoint<'a> {
        fn visit_tx_in(&mut self, vin: usize, tx_in: &bsl::TxIn) -> core::ops::ControlFlow<()> {
            let txid = Txid::from_slice(tx_in.prevout().txid()).unwrap();
            if self.txids.contains(&txid) {
                self.found = true;
            }
            core::ops::ControlFlow::Continue(())
        }

        fn visit_transaction(
            &mut self,
            tx: &bitcoin_slices::bsl::Transaction,
        ) -> core::ops::ControlFlow<()> {
            if self.found {
                self.txids.push(tx.txid().into());
                self.found = false;
            }
            core::ops::ControlFlow::Continue(())
        }
    }
    let mut visitor = TxContainingOutpoint {
        txids,
        found: false,
    };
    bsl::Block::visit(&b.0, &mut visitor).unwrap(); // TODO
}

/// Add txid to txids of transactions in block `b` containing `script_pubkey` in the outputs
fn find_txs_with_script_pubkey(script_pubkey: &ScriptBuf, b: SerBlock, txids: &mut Vec<Txid>) {
    struct TxContainingScript<'a> {
        txids: &'a mut Vec<Txid>,
        script_pubkey: &'a [u8],
        found: bool,
    }
    impl<'a> Visitor for TxContainingScript<'a> {
        fn visit_tx_out(&mut self, _vout: usize, tx_out: &bsl::TxOut) -> ControlFlow<()> {
            if self.script_pubkey == tx_out.script_pubkey() {
                self.found = true;
            }
            ControlFlow::Continue(())
        }

        fn visit_transaction(
            &mut self,
            tx: &bitcoin_slices::bsl::Transaction,
        ) -> core::ops::ControlFlow<()> {
            if self.found {
                self.txids.push(tx.txid().into());
                self.found = false;
            }
            core::ops::ControlFlow::Continue(())
        }
    }
    let mut visitor = TxContainingScript {
        txids,
        script_pubkey: script_pubkey.as_bytes(),
        found: false,
    };
    bsl::Block::visit(&b.0, &mut visitor).unwrap(); // TODO
}

fn index_block(block: &Block, height: u32) -> Result<IndexBlockResult, crate::Error> {
    let block_hash = block.block_hash();

    // # funding script_hashes, script_pubkeys in outputs
    let funding_sh: BTreeSet<ScriptHash> = block
        .txdata
        .iter()
        .flat_map(|tx| tx.output.iter())
        .map(|txout| script_hash(&txout.script_pubkey))
        .collect();

    // # spending script_hashes
    let spending_sh: BTreeSet<OutPoint> = block
        .txdata
        .iter()
        .flat_map(|tx| tx.input.iter())
        .map(|i| i.previous_output)
        .collect();

    Ok(IndexBlockResult {
        block_hash,
        height,
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

    let mut already_indexed = 0;
    for height in 0..chain_info.blocks {
        let hash = rpc::blockhashbyheight::call(height as usize).await?;
        let block = rpc::block::call(hash.block_hash).await?;
        let block_hash = block.block_hash();
        if db.is_block_hash_indexed(&block_hash) {
            // TODO load all already indexed instead and avoid the block rpc::block::call
            already_indexed += 1;
        } else {
            shared_state.update_cache(&block, Some(height)).await?;

            let index_res = index_block(&block, height)?;
            let db = db.clone();
            tokio::spawn(async move { db.write_hashes(index_res) });
        }
        if height % 10_000 == 0 {
            log::info!("indexed block {height} already_indexed:{already_indexed}")
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
