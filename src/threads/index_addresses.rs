use std::{
    collections::{BTreeSet, HashSet},
    fmt::Display,
    hash::Hasher,
    ops::ControlFlow,
    path::Path,
    sync::Arc,
    time::Duration,
};

use bitcoin::{hashes::Hash, Address, Block, BlockHash, OutPoint, Script, ScriptBuf, Txid};
use bitcoin_slices::{bsl, Visit, Visitor};
use fxhash::FxHasher64;
use rocksdb::{ColumnFamily, ColumnFamilyDescriptor, DBCompressionType, Options, WriteBatch, DB};

use crate::{
    error::Error,
    rpc::{self, block::SerBlock, headers::HeightTime},
    state::SharedState,
};

type ScriptHash = u64;
pub type Height = u32;

fn script_hash(script: &Script) -> ScriptHash {
    let mut hasher = FxHasher64::default();
    hasher.write(script.as_bytes());
    hasher.finish()
}

const BLOCK_HASH_CF: &str = "BLOCK_HASH_CF"; // BlockHash -> [] // indexed blocks
const FUNDING_CF: &str = "FUNDING_CF"; // hash(Script) || height -> []
const SPENDING_CF: &str = "SPENDING_CF"; // hash(prevout) || height -> []

const COLUMN_FAMILIES: &[&str] = &[BLOCK_HASH_CF, FUNDING_CF, SPENDING_CF];

#[derive(Debug)]
pub struct Database {
    db: DB,
}

impl Database {
    fn create_cf_descriptors() -> Vec<ColumnFamilyDescriptor> {
        COLUMN_FAMILIES
            .iter()
            .map(|&name| {
                let mut opts = Options::default();
                opts.set_compression_type(DBCompressionType::Zstd);
                ColumnFamilyDescriptor::new(name, opts)
            })
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
        self.db.cf_handle(FUNDING_CF).expect("missing FUNDING_CF")
    }

    fn spending_cf(&self) -> &ColumnFamily {
        self.db.cf_handle(SPENDING_CF).expect("missing SPENDING_CF")
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

    fn is_block_hash_indexed(&self, block_hash: &BlockHash) -> bool {
        self.db
            .get_pinned_cf(self.block_hash_cf(), block_hash)
            .unwrap()
            .is_some()
    }

    pub fn script_hash_heights(&self, script_pubkey: &Script) -> Vec<Height> {
        let script_hash = script_hash(script_pubkey).to_be_bytes();
        let mut starting = script_hash.to_vec();
        starting.extend(&[0xff; 4]);
        let mut result = vec![];

        for el in self.db.iterator_cf(
            self.funding_cf(),
            rocksdb::IteratorMode::From(&starting[..], rocksdb::Direction::Reverse),
        ) {
            let el = el.unwrap().0;
            if el.starts_with(&script_hash) {
                let height = u32::from_be_bytes(el[8..].try_into().unwrap());
                result.push(height);
            } else {
                break;
            }
            if result.len() > 9 {
                // TODO paging
                break;
            }
        }

        result
    }

    pub fn get_spending(&self, outpoint: &OutPoint) -> Option<Height> {
        let searched_key_start = outpoint_to_key_vec(outpoint);

        let (key, _val) = self
            .db
            .iterator_cf(
                self.spending_cf(),
                rocksdb::IteratorMode::From(&searched_key_start[..], rocksdb::Direction::Forward),
            )
            .next()
            .unwrap()
            .unwrap();

        if key[..8] == searched_key_start[..] {
            Some(u32::from_be_bytes((&key[8..]).try_into().unwrap()))
        } else {
            None
        }
    }

    pub fn _iter_spending(&self, _txid: Txid, _max: usize) -> Vec<Option<u32>> {
        // TODO implement iteration on outputs so that is only one db access for all the output of a tx, however it's not trivial
        todo!()
    }

    pub fn write_hashes(&self, index_res: IndexBlockResult) -> Result<(), Error> {
        let mut batch = WriteBatch::default();
        let height_bytes = index_res.height.to_be_bytes();

        let mut buffer = vec![];
        for script_hash in index_res.funding_sh {
            buffer.clear();
            buffer.extend(script_hash.to_be_bytes());
            buffer.extend(&height_bytes[..]);
            batch.put_cf(self.funding_cf(), &buffer, []);
        }
        for out_point in index_res.spending_sh {
            buffer.clear();
            outpoint_to_key(&out_point, &mut buffer);
            buffer.extend(&height_bytes[..]);
            batch.put_cf(self.spending_cf(), &buffer, []);
        }

        batch.put_cf(self.block_hash_cf(), index_res.block_hash, []);

        self.db.write(batch)?;
        Ok(())
    }
}

fn outpoint_hash(out_point: &OutPoint) -> u64 {
    u64::from_be_bytes((&out_point.txid[..8]).try_into().unwrap())
}
fn outpoint_to_key(out_point: &OutPoint, buffer: &mut Vec<u8>) {
    let mut val = outpoint_hash(out_point);
    val += out_point.vout as u64;
    buffer.extend(val.to_be_bytes());
}
pub fn outpoint_to_key_vec(out_point: &OutPoint) -> Vec<u8> {
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

#[derive(PartialEq, Eq, Debug)]
pub struct AddressSeen {
    pub funding: Funding,
    pub spending: Option<Spending>,
}

#[derive(PartialEq, Eq, Debug)]
pub struct Funding {
    pub out_point: OutPoint,
    pub block_hash: BlockHash,
    pub height_time: HeightTime,
}

impl AddressSeen {
    pub fn new(out_point: OutPoint, block_hash: BlockHash, height_time: HeightTime) -> Self {
        Self {
            funding: Funding {
                out_point,
                block_hash,
                height_time,
            },
            spending: None,
        }
    }
}

#[derive(PartialEq, Eq, Debug)]
pub struct Spending {
    pub txid: Txid,
    pub vin: usize,
    pub block_hash: BlockHash,
    pub height_time: HeightTime,
}

pub async fn address_seen(
    address: &Address,
    db: Arc<Database>,
    shared_state: Arc<SharedState>,
) -> Result<Vec<AddressSeen>, Error> {
    let script_pubkey = address.script_pubkey();
    let heights = db.script_hash_heights(&script_pubkey);
    let blocks = shared_state.blocks_from_heights(&heights).await?;
    let mut outpoints_with_script_pubkey = vec![];
    for (h, b) in blocks {
        let t = shared_state.height_time(h).await.unwrap();
        outpoints_with_script_pubkey.extend(
            find_outpoints_with_script_pubkey(&script_pubkey, b)
                .into_iter()
                .map(|e| (h, e, t)),
        );
    }

    let mut heights_with_spending = vec![];
    for (_, outpoint, _) in outpoints_with_script_pubkey.iter().take(10) {
        //TODO handle pagination?
        if let Some(h) = db.get_spending(outpoint) {
            heights_with_spending.push(h);
        }
    }
    let blocks = shared_state
        .blocks_from_heights(&heights_with_spending)
        .await?;
    let mut address_seen: Vec<_> = outpoints_with_script_pubkey
        .into_iter()
        .map(|(h, o, t)| AddressSeen::new(o, h, t))
        .collect();
    for (h, b) in blocks {
        let t = shared_state.height_time(h).await.unwrap();
        find_txids_with_prevout(h, b, t, &mut address_seen);
    }

    Ok(address_seen)
}
fn find_txids_with_prevout(
    h: BlockHash,
    b: SerBlock,
    t: HeightTime,
    address_seen: &mut Vec<AddressSeen>,
) {
    struct TxidContainingOutpoint<'a> {
        address_seen: &'a mut Vec<AddressSeen>,
        found: Option<(usize, usize)>,
        block_hash: BlockHash,
        height_time: HeightTime,
    }

    impl Visitor for TxidContainingOutpoint<'_> {
        fn visit_tx_in(&mut self, vin: usize, tx_in: &bsl::TxIn) -> core::ops::ControlFlow<()> {
            let current = tx_in.prevout().into();
            for (i, seen) in self.address_seen.iter().enumerate() {
                if seen.funding.out_point == current {
                    self.found = Some((i, vin));
                    break;
                }
            }
            core::ops::ControlFlow::Continue(())
        }

        fn visit_transaction(
            &mut self,
            tx: &bitcoin_slices::bsl::Transaction,
        ) -> core::ops::ControlFlow<()> {
            if let Some((i, vin)) = self.found.take() {
                self.address_seen.get_mut(i).unwrap().spending = Some(Spending {
                    txid: Txid::from_raw_hash(tx.txid()),
                    vin,
                    block_hash: self.block_hash,
                    height_time: self.height_time,
                });
            }
            core::ops::ControlFlow::Continue(())
        }
    }
    let mut visitor = TxidContainingOutpoint {
        address_seen,
        found: None,
        block_hash: h,
        height_time: t,
    };
    bsl::Block::visit(&b.0, &mut visitor).unwrap();
}

/// Add txid to txids of transactions in block `b` containing `script_pubkey` in the outputs
fn find_outpoints_with_script_pubkey(script_pubkey: &ScriptBuf, b: SerBlock) -> Vec<OutPoint> {
    struct TxContainingScript<'a> {
        outpoints: Vec<OutPoint>,
        script_pubkey: &'a [u8],
        current_tx_matching_vouts: Vec<u32>,
    }
    impl Visitor for TxContainingScript<'_> {
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

pub fn index_block(block: &Block, height: u32) -> Result<IndexBlockResult, crate::Error> {
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

    let indexed_block_hash = db.indexed_block_hash();
    log::info!("already_indexed:{}", indexed_block_hash.len());

    for height in 0.. {
        let block_hash = match shared_state.height_to_hash(height).await {
            Some(hash) if hash != BlockHash::all_zeros() => hash,
            _ => {
                log::info!("stopping initial block indexing");
                break;
            }
        };
        if indexed_block_hash.contains(&block_hash) || db.is_block_hash_indexed(&block_hash) {
            // there are 2 checks because the first is fast the second is fresher
            continue;
        }
        if height % 5_000 == 0 {
            log::info!("indexed block {height} ")
        }

        let block = loop {
            match rpc::block::call(block_hash).await {
                Ok(block) => break block,
                Err(e) => {
                    log::warn!("Cannot download block: {block_hash} {e}");
                    tokio::time::sleep(Duration::from_secs(1)).await
                }
            }
        };
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

    #[test]
    fn test_endianness_ordering() {
        let expected: Vec<_> = (0u32..1000).collect();
        let mut v: Vec<_> = expected.iter().rev().map(|e| e.to_be_bytes()).collect();
        v.sort();
        let sorted_v: Vec<_> = v.into_iter().map(u32::from_be_bytes).collect();
        assert_eq!(expected, sorted_v);

        let mut v: Vec<_> = (0u32..1000).rev().map(|e| e.to_le_bytes()).collect();
        v.sort();
        let sorted_v: Vec<_> = v.into_iter().map(u32::from_le_bytes).collect();
        assert_ne!(expected, sorted_v, "little endian bytes sort");
    }
}
