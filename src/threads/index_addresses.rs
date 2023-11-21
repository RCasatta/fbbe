use std::{
    collections::{BTreeSet, HashSet},
    fmt::Display,
    hash::Hasher,
    ops::ControlFlow,
    path::Path,
    sync::Arc,
};

use bitcoin::{hashes::Hash, Address, Block, BlockHash, OutPoint, Script, ScriptBuf, Txid};
use bitcoin_slices::{bsl, Visit, Visitor};
use fxhash::FxHasher64;
use rocksdb::{ColumnFamily, ColumnFamilyDescriptor, Options, WriteBatch, DB};

use crate::{
    error::Error,
    rpc::{self, block::SerBlock},
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

    fn funding_cf(&self) -> &ColumnFamily {
        self.db.cf_handle("FUNDING_CF").expect("missing FUNDING_CF")
    }

    fn spending_cf(&self) -> &ColumnFamily {
        self.db
            .cf_handle("SPENDING_CF")
            .expect("missing SPENDING_CF")
    }

    pub fn indexed_block_hash(&self) -> HashSet<BlockHash> {
        let mut result = HashSet::new();
        for el in self
            .db
            .iterator_cf(self.block_hash_cf(), rocksdb::IteratorMode::Start)
        {
            let el = el.unwrap().0;
            result.insert(BlockHash::from_slice(&el[..]).unwrap());
        }
        result
    }

    pub fn script_hash_heights(&self, script_pubkey: &Script) -> Vec<Height> {
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

    pub fn get_spending(&self, outpoint: &OutPoint) -> Option<Height> {
        let searched_key_start = outpoint_to_key_vec(&outpoint);

        let (key, _val) = self
            .db
            .iterator_cf(
                self.spending_cf(),
                rocksdb::IteratorMode::From(&searched_key_start[..], rocksdb::Direction::Forward),
            )
            .next()
            .unwrap()
            .unwrap();

        if &key[..8] == &searched_key_start[..] {
            Some(u32::from_le_bytes((&key[8..]).try_into().unwrap()))
        } else {
            None
        }
    }

    pub fn _iter_spending(&self, _txid: Txid, _max: usize) -> Vec<Option<u32>> {
        // TODO implement iteration on outputs so that is only one db access for all the output of a tx, however it's not trivial
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
            outpoint_to_key(&out_point, &mut buffer);
            buffer.extend(&height_bytes[..]);
            batch.put_cf(self.spending_cf(), &buffer, &[]);
        }

        batch.put_cf(self.block_hash_cf(), index_res.block_hash, &[]);

        self.db.write(batch).unwrap();
    }
}

fn outpoint_hash(out_point: &OutPoint) -> u64 {
    u64::from_le_bytes((&out_point.txid[..8]).try_into().unwrap())
}
fn outpoint_to_key(out_point: &OutPoint, buffer: &mut Vec<u8>) {
    let mut val = outpoint_hash(out_point);
    val += out_point.vout as u64;
    buffer.extend(val.to_le_bytes());
}
fn outpoint_to_key_vec(out_point: &OutPoint) -> Vec<u8> {
    let mut vec = Vec::with_capacity(8);
    outpoint_to_key(out_point, &mut vec);
    vec
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
    let mut outpoints_with_script_pubkey = vec![];
    for (_, b) in blocks {
        outpoints_with_script_pubkey.extend(find_outpoints_with_script_pubkey(&script_pubkey, b));
    }

    let mut heights_with_spending = vec![];
    for outpoint in outpoints_with_script_pubkey.iter() {
        if let Some(h) = db.get_spending(outpoint) {
            heights_with_spending.push(h);
        }
    }
    let blocks = shared_state
        .blocks_from_heights(&heights_with_spending)
        .await?;
    let mut txids: Vec<_> = outpoints_with_script_pubkey
        .iter()
        .map(|o| o.txid)
        .collect();
    for (_, b) in blocks {
        txids.extend(find_txids_with_prevout(b, &outpoints_with_script_pubkey));
    }

    Ok(txids)
}
fn find_txids_with_prevout(b: SerBlock, outpoints: &[OutPoint]) -> Vec<Txid> {
    struct TxidContainingOutpoint<'a> {
        outpoints: &'a [OutPoint],
        found: bool,
        result: Vec<Txid>,
    }

    impl<'a> Visitor for TxidContainingOutpoint<'a> {
        fn visit_tx_in(&mut self, _vin: usize, tx_in: &bsl::TxIn) -> core::ops::ControlFlow<()> {
            if self.outpoints.contains(&tx_in.prevout().into()) {
                self.found = true;
            }
            core::ops::ControlFlow::Continue(())
        }

        fn visit_transaction(
            &mut self,
            tx: &bitcoin_slices::bsl::Transaction,
        ) -> core::ops::ControlFlow<()> {
            if self.found {
                self.result.push(tx.txid().into());
                self.found = false;
            }
            core::ops::ControlFlow::Continue(())
        }
    }
    let mut visitor = TxidContainingOutpoint {
        outpoints,
        found: false,
        result: vec![],
    };
    bsl::Block::visit(&b.0, &mut visitor).unwrap();
    visitor.result
}

/// Add txid to txids of transactions in block `b` containing `script_pubkey` in the outputs
fn find_outpoints_with_script_pubkey(script_pubkey: &ScriptBuf, b: SerBlock) -> Vec<OutPoint> {
    struct TxContainingScript<'a> {
        outpoints: Vec<OutPoint>,
        script_pubkey: &'a [u8],
        current_tx_matching_vouts: Vec<u32>,
    }
    impl<'a> Visitor for TxContainingScript<'a> {
        fn visit_tx_out(&mut self, vout: usize, tx_out: &bsl::TxOut) -> ControlFlow<()> {
            if self.script_pubkey == tx_out.script_pubkey() {
                self.current_tx_matching_vouts.push(vout as u32);
            }
            ControlFlow::Continue(())
        }

        fn visit_transaction(
            &mut self,
            tx: &bitcoin_slices::bsl::Transaction,
        ) -> core::ops::ControlFlow<()> {
            if !self.current_tx_matching_vouts.is_empty() {
                let txid: Txid = tx.txid().into();
                for vout in self.current_tx_matching_vouts.iter() {
                    self.outpoints.push(OutPoint { txid, vout: *vout });
                }
                self.current_tx_matching_vouts.clear();
            }
            core::ops::ControlFlow::Continue(())
        }
    }
    let mut visitor = TxContainingScript {
        script_pubkey: script_pubkey.as_bytes(),
        outpoints: vec![],
        current_tx_matching_vouts: vec![],
    };
    bsl::Block::visit(&b.0, &mut visitor).unwrap(); // TODO
    visitor.outpoints
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

pub(crate) async fn index_addresses_infallible(db: Arc<Database>, shared_state: Arc<SharedState>) {
    if let Err(e) = index_addresses(db, shared_state).await {
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

async fn index_addresses(db: Arc<Database>, shared_state: Arc<SharedState>) -> Result<(), Error> {
    log::info!("Starting index_addresses");

    let mut already_indexed = 0;

    let indexed_block_hash = db.indexed_block_hash();

    for height in 0.. {
        if height % 10_000 == 0 {
            log::info!("indexed block {height} already_indexed:{already_indexed}")
        }

        let block_hash = match shared_state
            .height_to_hash
            .lock()
            .await
            .get(height as usize)
        {
            Some(hash) => *hash,
            None => break,
        };
        if indexed_block_hash.contains(&block_hash) {
            already_indexed += 1;
            continue;
        }
        let block = rpc::block::call(block_hash).await?;

        // shared_state.update_cache(&block, Some(height)).await?;

        let index_res = index_block(&block, height)?;
        let db = db.clone();
        tokio::spawn(async move { db.write_hashes(index_res) });
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
