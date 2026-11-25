use std::{
    collections::{BTreeSet, HashMap, HashSet, VecDeque},
    fmt::Display,
    ops::ControlFlow,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, MutexGuard,
    },
    time::{Duration, Instant},
};

use bitcoin::{
    hashes::{siphash24, Hash},
    Address, Block, BlockHash, OutPoint, Script, ScriptBuf, TxOut, Txid,
};
use bitcoin_slices::{bsl, Visit, Visitor};
use rocksdb::{properties, MergeOperands};
use rocksdb::{ColumnFamily, ColumnFamilyDescriptor, DBCompressionType, Options, WriteBatch, DB};

use crate::{
    cache_counter,
    error::Error,
    rpc::{self, block::SerBlock, headers::HeightTime},
    state::SharedState,
};

type ScriptHash = u64;
pub type Height = u32;

const BLOCK_HASH_CF: &str = "BLOCK_HASH_CF"; // BlockHash -> [] // indexed blocks
const FUNDING_CF: &str = "FUNDING_CF"; // hash(Script) || height -> []
const SPENDING_CF: &str = "SPENDING_CF"; // hash(prevout) || height -> []
const FEE_CF: &str = "FEE_CF"; // txid[..12] -> fee
const BALANCE_CF: &str = "BALANCE_CF"; // hash(Script) -> confirmed balance, see `merge_sum`
const META_CF: &str = "META_CF"; // the keys below

/// Format version of the database, `u32` big endian
const FORMAT_VERSION_KEY: &[u8] = b"V";
/// Key of the script hash, 16 random bytes chosen when the database is created
const SCRIPT_HASH_KEY: &[u8] = b"K";
/// Height (`u32` big endian) and hash of the last block counted in `BALANCE_CF`
const BALANCE_TIP_KEY: &[u8] = b"T";

/// 1. Script hashes computed with keyed SipHash-2-4, confirmed balances in `BALANCE_CF`.
///
/// Databases created before format versions existed, without `META_CF`, are refused: their
/// script hashes are unkeyed and they have no balances, they must be reindexed.
const FORMAT_VERSION: u32 = 1;

const COLUMN_FAMILIES: &[&str] = &[
    BLOCK_HASH_CF,
    FUNDING_CF,
    SPENDING_CF,
    FEE_CF,
    BALANCE_CF,
    META_CF,
];

/// Blocks this deep or less are kept in memory and reach `BALANCE_CF` only when buried deeper,
/// so that a reorg never needs undoing what is in the db. A deeper reorg is detected, it makes
/// the balances unavailable until a reindex.
pub const REORG_DEPTH: usize = 6;

/// How scripts are hashed in the keys of the database, it can't change without reindexing.
///
/// SipHash-2-4 keyed with random bytes stored in the database, so that nobody can craft a
/// script colliding with a given address, which would alter its balance. It comes with
/// `bitcoin_hashes`, and std's `DefaultHasher` can't be used: its algorithm may change
/// between Rust releases.
#[derive(Debug, Clone, Copy)]
struct ScriptHasher {
    k0: u64,
    k1: u64,
}

impl ScriptHasher {
    fn new(key: [u8; 16]) -> Self {
        let (k0, k1) = key.split_at(8);
        Self {
            k0: u64::from_le_bytes(k0.try_into().expect("8 bytes")),
            k1: u64::from_le_bytes(k1.try_into().expect("8 bytes")),
        }
    }

    fn hash(&self, script: &Script) -> ScriptHash {
        siphash24::Hash::hash_to_u64_with_keys(self.k0, self.k1, script.as_bytes())
    }
}

/// The balance change of each script in a block, in satoshi
pub struct BlockDeltas {
    height: Height,
    block_hash: BlockHash,
    prev_blockhash: BlockHash,
    deltas: HashMap<ScriptHash, i64>,
}

/// The last block counted in `BALANCE_CF`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BalanceTip {
    pub height: Height,
    pub block_hash: BlockHash,
}

impl BalanceTip {
    fn to_bytes(self) -> [u8; 36] {
        let mut bytes = [0u8; 36];
        bytes[..4].copy_from_slice(&self.height.to_be_bytes());
        bytes[4..].copy_from_slice(self.block_hash.as_byte_array());
        bytes
    }

    fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let height = u32::from_be_bytes(bytes.get(..4)?.try_into().ok()?);
        let block_hash = BlockHash::from_slice(bytes.get(4..)?).ok()?;
        Some(Self { height, block_hash })
    }
}

/// The blocks not yet in `BALANCE_CF`
#[derive(Debug, Default)]
struct Recent {
    /// `None` until the genesis block reaches `BALANCE_CF`
    tip: Option<BalanceTip>,
    /// consecutive blocks following `tip`, at most [`REORG_DEPTH`]
    blocks: VecDeque<BlockDeltas>,
    /// a reorg replaced blocks already in `BALANCE_CF`
    reindex_needed: bool,
}

impl std::fmt::Debug for BlockDeltas {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "BlockDeltas({} {})", self.height, self.block_hash)
    }
}

impl Recent {
    /// Height and parent hash of the block that can be pushed next
    fn next(&self) -> (Height, BlockHash) {
        match (self.blocks.back(), self.tip) {
            (Some(last), _) => (last.height + 1, last.block_hash),
            (None, Some(tip)) => (tip.height + 1, tip.block_hash),
            (None, None) => (0, BlockHash::all_zeros()),
        }
    }
}

#[derive(Debug)]
pub struct Database {
    db: DB,
    hasher: ScriptHasher,
    /// The initial indexing reached the tip, every funding height is in the db
    synced: AtomicBool,
    /// Held while `BALANCE_CF` is written, so that a balance read sees the db and the recent
    /// blocks at the same point
    recent: Mutex<Recent>,
}

impl Database {
    fn create_cf_descriptors() -> Vec<ColumnFamilyDescriptor> {
        COLUMN_FAMILIES
            .iter()
            .map(|&name| {
                let mut opts = Options::default();
                opts.set_compression_type(DBCompressionType::Zstd);
                if name == BALANCE_CF {
                    opts.set_merge_operator_associative("sum", merge_sum);
                }
                ColumnFamilyDescriptor::new(name, opts)
            })
            .collect()
    }

    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self, Error> {
        let path = path.as_ref();
        // an error here means there is no database yet
        if let Ok(cfs) = DB::list_cf(&Options::default(), path) {
            if !cfs.iter().any(|cf| cf == META_CF) {
                return Err(Error::AddrIndex(format!(
                    "{} was created by a previous release, its format is no longer supported: \
                     point --addr-index-path to an empty directory to reindex",
                    path.display()
                )));
            }
        }

        let mut db_opts = Options::default();

        db_opts.create_if_missing(true);
        db_opts.create_missing_column_families(true);

        let db = DB::open_cf_descriptors(&db_opts, path, Self::create_cf_descriptors())?;
        let hasher = get_or_init_script_hasher(&db)?;
        let meta_cf = db.cf_handle(META_CF).expect("missing META_CF");
        let tip = match db.get_cf(meta_cf, BALANCE_TIP_KEY)? {
            None => None,
            Some(bytes) => Some(
                BalanceTip::from_bytes(&bytes)
                    .ok_or_else(|| Error::AddrIndex("invalid balance tip".into()))?,
            ),
        };
        log::info!("address index balance tip: {tip:?}");
        Ok(Self {
            db,
            hasher,
            synced: AtomicBool::new(false),
            recent: Mutex::new(Recent {
                tip,
                ..Default::default()
            }),
        })
    }

    fn recent(&self) -> MutexGuard<'_, Recent> {
        self.recent.lock().expect("poisoned")
    }

    fn balance_cf(&self) -> &ColumnFamily {
        self.db.cf_handle(BALANCE_CF).expect("missing BALANCE_CF")
    }

    fn meta_cf(&self) -> &ColumnFamily {
        self.db.cf_handle(META_CF).expect("missing META_CF")
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

    fn fee_cf(&self) -> &ColumnFamily {
        self.db.cf_handle(FEE_CF).expect("missing FEE_CF")
    }

    /// The balance change of each script in `block`, `spent` are the outputs spent by each of
    /// its transactions, see [`rpc::spenttxouts::call`]
    pub fn block_deltas(
        &self,
        block: &Block,
        height: Height,
        spent: &[Vec<TxOut>],
    ) -> Result<BlockDeltas, Error> {
        let block_hash = block.block_hash();
        if spent.len() != block.txdata.len()
            || block
                .txdata
                .iter()
                .zip(spent)
                .any(|(tx, spent)| !tx.is_coinbase() && tx.input.len() != spent.len())
        {
            return Err(Error::SpentTxOutsMismatch(block_hash));
        }
        let mut deltas: HashMap<ScriptHash, i64> = HashMap::new();
        for tx in block.txdata.iter() {
            for output in tx.output.iter() {
                *deltas
                    .entry(self.hasher.hash(&output.script_pubkey))
                    .or_default() += output.value.to_sat() as i64;
            }
        }
        for output in spent.iter().flatten() {
            *deltas
                .entry(self.hasher.hash(&output.script_pubkey))
                .or_default() -= output.value.to_sat() as i64;
        }
        // eg. a script receiving and spending the same amount, or zero value OP_RETURNs
        deltas.retain(|_, delta| *delta != 0);
        Ok(BlockDeltas {
            height,
            block_hash,
            prev_blockhash: block.header.prev_blockhash,
            deltas,
        })
    }

    /// Height of the next block to push, the one after the last pushed
    pub fn next_balance_height(&self) -> Height {
        self.recent().next().0
    }

    /// Adds `block` to the recent ones, if it extends them, and writes in `BALANCE_CF` the
    /// recent blocks buried more than [`REORG_DEPTH`]. Returns `false` if `block` doesn't
    /// extend the recent blocks, so it isn't counted twice or out of the chain.
    pub fn push_block(&self, block: BlockDeltas) -> Result<bool, Error> {
        let mut recent = self.recent();
        if (block.height, block.prev_blockhash) != recent.next() {
            return Ok(false);
        }
        recent.blocks.push_back(block);
        while recent.blocks.len() > REORG_DEPTH {
            let oldest = recent.blocks.front().expect("not empty");
            let tip = BalanceTip {
                height: oldest.height,
                block_hash: oldest.block_hash,
            };
            let mut batch = WriteBatch::default();
            for (script_hash, delta) in oldest.deltas.iter() {
                batch.merge_cf(
                    self.balance_cf(),
                    script_hash.to_be_bytes(),
                    encode_delta(*delta),
                );
            }
            // in the same batch, so a block is counted once even if the process is killed
            batch.put_cf(self.meta_cf(), BALANCE_TIP_KEY, tip.to_bytes());
            self.db.write(batch)?;
            recent.blocks.pop_front();
            recent.tip = Some(tip);
        }
        Ok(true)
    }

    /// The last block in `BALANCE_CF` and the recent blocks, by height and hash
    pub fn balance_chain(&self) -> (Option<BalanceTip>, Vec<(Height, BlockHash)>) {
        let recent = self.recent();
        let blocks = recent
            .blocks
            .iter()
            .map(|b| (b.height, b.block_hash))
            .collect();
        (recent.tip, blocks)
    }

    /// Drops the recent blocks from `height` on, replaced by a reorg
    pub fn truncate_recent(&self, height: Height) {
        self.recent().blocks.retain(|b| b.height < height);
    }

    /// A reorg replaced a block already in `BALANCE_CF`
    pub fn set_reindex_needed(&self) {
        self.recent().reindex_needed = true;
    }

    /// The confirmed balance of `script_pubkey`
    pub fn balance(&self, script_pubkey: &Script) -> Result<Balance, Error> {
        if !self.is_synced() {
            return Ok(Balance::IndexSyncing);
        }
        let script_hash = self.hasher.hash(script_pubkey);
        let recent = self.recent();
        if recent.reindex_needed {
            return Ok(Balance::ReindexNeeded);
        }
        let stored = match self
            .db
            .get_pinned_cf(self.balance_cf(), script_hash.to_be_bytes())?
        {
            None => 0,
            Some(value) => decode_delta(&value)
                .ok_or_else(|| Error::AddrIndex("invalid balance value".into()))?,
        };
        let recent_deltas: i64 = recent
            .blocks
            .iter()
            .filter_map(|b| b.deltas.get(&script_hash))
            .sum();
        match u64::try_from(stored + recent_deltas) {
            Ok(sat) => Ok(Balance::Known(sat)),
            Err(_) => {
                log::error!("negative balance for {script_pubkey}, the index is corrupted");
                Ok(Balance::ReindexNeeded)
            }
        }
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

    /// Returns at most `max` heights of the blocks funding `script_pubkey`, the latest first
    pub fn script_hash_latest_heights(&self, script_pubkey: &Script, max: usize) -> Vec<Height> {
        let script_hash = self.hasher.hash(script_pubkey).to_be_bytes();
        let mut starting = script_hash.to_vec();
        starting.extend(&[0xff; 4]);

        self.db
            .iterator_cf(
                self.funding_cf(),
                rocksdb::IteratorMode::From(&starting[..], rocksdb::Direction::Reverse),
            )
            .map(|el| el.unwrap().0)
            .take_while(|key| key.starts_with(&script_hash))
            .take(max)
            .map(|key| u32::from_be_bytes(key[8..].try_into().unwrap()))
            .collect()
    }

    pub fn is_synced(&self) -> bool {
        self.synced.load(Ordering::Relaxed)
    }

    pub fn get_spending(&self, outpoint: &OutPoint) -> Option<Height> {
        let searched_key_start = outpoint_to_key_vec(outpoint);

        let mut iter = self.db.iterator_cf(
            self.spending_cf(),
            rocksdb::IteratorMode::From(&searched_key_start[..], rocksdb::Direction::Forward),
        );
        let (key, _val) = iter.next()?.unwrap();

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

    pub fn get_fee(&self, txid: &Txid) -> Option<u64> {
        let key = txid_fee_key(txid);
        let value = self.db.get_pinned_cf(self.fee_cf(), key).unwrap();
        cache_counter("fee", value.is_some());
        let value = value?;
        if value.len() != 8 {
            return None;
        }
        Some(u64::from_be_bytes(value[..].try_into().unwrap()))
    }

    pub fn put_fee(&self, txid: &Txid, fee: u64) -> Result<(), rocksdb::Error> {
        let key = txid_fee_key(txid);
        self.db.put_cf(self.fee_cf(), key, fee.to_be_bytes())
    }

    pub fn estimated_num_keys(&self) -> Result<Vec<(&'static str, u64)>, rocksdb::Error> {
        let cfs = [
            ("block_hash", self.block_hash_cf()),
            ("funding", self.funding_cf()),
            ("spending", self.spending_cf()),
            ("fee", self.fee_cf()),
            ("balance", self.balance_cf()),
        ];

        cfs.into_iter()
            .map(|(name, cf)| {
                let value = self
                    .db
                    .property_int_value_cf(cf, properties::ESTIMATE_NUM_KEYS)?
                    .unwrap_or(0);
                Ok((name, value))
            })
            .collect()
    }

    pub fn index_block(&self, block: &Block, height: Height) -> IndexBlockResult {
        // # funding script_hashes, script_pubkeys in outputs
        let funding_sh: BTreeSet<ScriptHash> = block
            .txdata
            .iter()
            .flat_map(|tx| tx.output.iter())
            .map(|txout| self.hasher.hash(&txout.script_pubkey))
            .collect();

        // # spending script_hashes
        let spending_sh: BTreeSet<OutPoint> = block
            .txdata
            .iter()
            .flat_map(|tx| tx.input.iter())
            .map(|i| i.previous_output)
            .collect();

        IndexBlockResult {
            block_hash: block.block_hash(),
            height,
            funding_sh,
            spending_sh,
        }
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

fn txid_fee_key(txid: &Txid) -> [u8; 12] {
    txid.as_byte_array()[..12]
        .try_into()
        .expect("slice right length")
}

/// Reads the format version and the script hash key, or writes them in a new database.
fn get_or_init_script_hasher(db: &DB) -> Result<ScriptHasher, Error> {
    let cf = db.cf_handle(META_CF).expect("missing META_CF");
    match db.get_cf(cf, FORMAT_VERSION_KEY)? {
        None => {
            // META_CF was just created, a previous first start may have been interrupted
            // before this point, but nothing is indexed without the key
            let mut key = [0u8; 16];
            getrandom::getrandom(&mut key)
                .map_err(|e| Error::AddrIndex(format!("cannot generate the key: {e}")))?;
            let mut batch = WriteBatch::default();
            batch.put_cf(cf, FORMAT_VERSION_KEY, FORMAT_VERSION.to_be_bytes());
            batch.put_cf(cf, SCRIPT_HASH_KEY, key);
            db.write(batch)?;
            Ok(ScriptHasher::new(key))
        }
        Some(version) if version[..] == FORMAT_VERSION.to_be_bytes() => {
            let key = db
                .get_cf(cf, SCRIPT_HASH_KEY)?
                .and_then(|key| <[u8; 16]>::try_from(key).ok())
                .ok_or_else(|| Error::AddrIndex("missing or invalid script hash key".into()))?;
            Ok(ScriptHasher::new(key))
        }
        Some(version) => Err(Error::AddrIndex(format!(
            "unsupported format version {version:?}, this release supports {FORMAT_VERSION}"
        ))),
    }
}

/// Merges the balance changes of a script: the value and the operands are zigzag varints, see
/// [`encode_delta`], the result is their sum. Returns `None`, failing the merge, on invalid data.
fn merge_sum(_key: &[u8], existing: Option<&[u8]>, operands: &MergeOperands) -> Option<Vec<u8>> {
    let mut sum = match existing {
        Some(value) => decode_delta(value)?,
        None => 0,
    };
    for operand in operands.iter() {
        sum = sum.checked_add(decode_delta(operand)?)?;
    }
    Some(encode_delta(sum))
}

/// LEB128 varint of the zigzag encoding of `value`, small values in few bytes whatever the sign
fn encode_delta(value: i64) -> Vec<u8> {
    let mut zigzag = ((value << 1) ^ (value >> 63)) as u64;
    let mut bytes = Vec::with_capacity(10);
    loop {
        let byte = (zigzag & 0x7f) as u8;
        zigzag >>= 7;
        if zigzag == 0 {
            bytes.push(byte);
            return bytes;
        }
        bytes.push(byte | 0x80);
    }
}

fn decode_delta(bytes: &[u8]) -> Option<i64> {
    let mut zigzag = 0u64;
    for (i, byte) in bytes.iter().enumerate() {
        if i >= 10 {
            return None;
        }
        zigzag |= ((byte & 0x7f) as u64) << (7 * i);
        if byte & 0x80 == 0 {
            if i + 1 != bytes.len() {
                return None;
            }
            return Some((zigzag >> 1) as i64 ^ -((zigzag & 1) as i64));
        }
    }
    None
}
pub struct IndexBlockResult {
    block_hash: BlockHash,
    height: Height,

    funding_sh: BTreeSet<ScriptHash>,
    spending_sh: BTreeSet<OutPoint>,
}

/// A page of the outputs of an address funded in its latest funding block, with its balance
pub struct AddressOutputs {
    /// at most [`OUTPUTS_PER_PAGE`] outputs, starting at `page * OUTPUTS_PER_PAGE`
    pub seen: Vec<AddressSeen>,
    /// number of outputs paying the address in its latest funding block, over all pages
    pub total: usize,
    pub page: usize,
    pub balance: Balance,
}

/// Outputs listed in an address page, each one is checked for spending
pub const OUTPUTS_PER_PAGE: usize = MAX_SPENDING_CHECKS;

/// The confirmed balance, from the running total of each script in `BALANCE_CF` plus the
/// recent blocks
#[derive(PartialEq, Eq, Debug)]
pub enum Balance {
    /// in satoshi
    Known(u64),
    IndexSyncing,
    /// a reorg deeper than [`REORG_DEPTH`] replaced blocks already counted
    ReindexNeeded,
}

#[derive(PartialEq, Eq, Debug)]
pub struct AddressSeen {
    pub funding: Funding,
    pub spending: Option<Spending>,
}

#[derive(PartialEq, Eq, Debug)]
pub struct Funding {
    pub out_point: OutPoint,
    /// in satoshi
    pub value: u64,
    pub block_hash: BlockHash,
    pub height_time: HeightTime,
}

impl AddressSeen {
    pub fn new(
        out_point: OutPoint,
        value: u64,
        block_hash: BlockHash,
        height_time: HeightTime,
    ) -> Self {
        Self {
            funding: Funding {
                out_point,
                value,
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
    page: usize,
) -> Result<AddressOutputs, Error> {
    let script_pubkey = address.script_pubkey();
    let heights = db.script_hash_latest_heights(&script_pubkey, 1);
    let blocks = shared_state.blocks_from_heights(&heights).await?;
    let start = page * OUTPUTS_PER_PAGE;
    let mut outpoints_with_script_pubkey = vec![];
    let mut total = 0;
    for (h, b) in blocks {
        let t = shared_state.height_time(h).await.unwrap();
        let (outpoints, count) =
            find_outpoints_with_script_pubkey(&script_pubkey, b, start, OUTPUTS_PER_PAGE);
        total += count;
        outpoints_with_script_pubkey.extend(outpoints.into_iter().map(|e| (h, e, t)));
    }
    if page > 0 && start >= total {
        return Err(Error::InvalidPageNumber);
    }

    let mut heights_with_spending = BTreeSet::new();
    for (_, (outpoint, _), _) in outpoints_with_script_pubkey.iter() {
        if let Some(h) = db.get_spending(outpoint) {
            heights_with_spending.insert(h);
        }
    }
    let heights_with_spending: Vec<_> = heights_with_spending.into_iter().collect();
    let blocks = shared_state
        .blocks_from_heights(&heights_with_spending)
        .await?;
    let mut address_seen: Vec<_> = outpoints_with_script_pubkey
        .into_iter()
        .map(|(h, (o, value), t)| AddressSeen::new(o, value, h, t))
        .collect();
    for (h, b) in blocks {
        let t = shared_state.height_time(h).await.unwrap();
        find_txids_with_prevout(h, b, t, &mut address_seen);
    }

    let balance = db.balance(&script_pubkey)?;

    Ok(AddressOutputs {
        seen: address_seen,
        total,
        page,
        balance,
    })
}

/// Outputs checked for spending, each could require downloading a block
const MAX_SPENDING_CHECKS: usize = 10;
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

/// Returns the outpoints paying to `script_pubkey` in the block, with their value in satoshi,
/// skipping the first `skip` and keeping at most `take`, and the count of all the matching ones.
/// Only the requested window is kept in memory, an address could have thousands of outputs.
fn find_outpoints_with_script_pubkey(
    script_pubkey: &ScriptBuf,
    b: SerBlock,
    skip: usize,
    take: usize,
) -> (Vec<(OutPoint, u64)>, usize) {
    struct TxContainingScript<'a> {
        outpoints: Vec<(OutPoint, u64)>,
        count: usize,
        skip: usize,
        take: usize,
        script_pubkey: &'a [u8],
        current_tx_matching_vouts: Vec<(u32, u64)>,
    }
    impl Visitor for TxContainingScript<'_> {
        fn visit_tx_out(&mut self, vout: usize, tx_out: &bsl::TxOut) -> ControlFlow<()> {
            if self.script_pubkey == tx_out.script_pubkey() {
                self.current_tx_matching_vouts
                    .push((vout as u32, tx_out.value()));
            }
            ControlFlow::Continue(())
        }

        fn visit_transaction(
            &mut self,
            tx: &bitcoin_slices::bsl::Transaction,
        ) -> core::ops::ControlFlow<()> {
            if !self.current_tx_matching_vouts.is_empty() {
                // the txid hashing is skipped for matches outside the window
                let end = self.skip.saturating_add(self.take);
                let matching = self.current_tx_matching_vouts.len();
                if self.count + matching <= self.skip || self.count >= end {
                    self.count += matching;
                    self.current_tx_matching_vouts.clear();
                    return core::ops::ControlFlow::Continue(());
                }
                let txid: Txid = tx.txid().into();
                for (vout, value) in self.current_tx_matching_vouts.iter() {
                    if self.count >= self.skip && self.outpoints.len() < self.take {
                        self.outpoints
                            .push((OutPoint { txid, vout: *vout }, *value));
                    }
                    self.count += 1;
                }
                self.current_tx_matching_vouts.clear();
            }
            core::ops::ControlFlow::Continue(())
        }
    }
    let mut visitor = TxContainingScript {
        script_pubkey: script_pubkey.as_bytes(),
        outpoints: Vec::with_capacity(take.min(OUTPUTS_PER_PAGE)),
        count: 0,
        skip,
        take,
        current_tx_matching_vouts: vec![],
    };
    bsl::Block::visit(&b.0, &mut visitor).unwrap(); // TODO
    (visitor.outpoints, visitor.count)
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
    // blocks from here are counted in the balances, the ones before already are
    let balance_from = db.next_balance_height();
    let mut balance_extends = true;
    let mut progress = Progress::new(Instant::now());

    for height in 0.. {
        let block_hash = match shared_state.height_to_hash(height).await {
            Some(hash) if hash != BlockHash::all_zeros() => hash,
            _ => {
                log::info!(
                    "stopping initial block indexing, {} blocks indexed in {}",
                    progress.total_blocks,
                    format_duration(progress.start.elapsed())
                );
                db.synced.store(true, Ordering::Relaxed);
                break;
            }
        };
        // there are 2 checks because the first is fast the second is fresher
        let index_hashes =
            !indexed_block_hash.contains(&block_hash) && !db.is_block_hash_indexed(&block_hash);
        let index_balance = balance_extends && height >= balance_from;
        if !index_hashes && !index_balance {
            continue;
        }
        let (block, spent) = fetch_block(block_hash, index_balance).await;
        let tip = shared_state.chain_info.lock().await.blocks;
        if let Some(line) = progress.block(Instant::now(), height, tip, &block) {
            log::info!("{line}");
        }
        if index_hashes {
            let index_res = db.index_block(&block, height);
            let db = db.clone();
            tokio::spawn(async move { db.write_hashes(index_res) });
        }
        if let Some(spent) = spent {
            let deltas = db.block_deltas(&block, height, &spent)?;
            if !db.push_block(deltas)? {
                // the chain changed under the balances, see `update_balance` once synced
                log::warn!("block {height} {block_hash} doesn't extend the balances");
                balance_extends = false;
            }
        }
    }
    Ok(())
}

/// Progress of the initial indexing logged at most once every this interval
const PROGRESS_INTERVAL: Duration = Duration::from_secs(60);

/// Counts the blocks indexed since the last progress line, to log the current rates: early
/// blocks are tiny, so the time left is an estimate at the current rate, not a forecast.
struct Progress {
    start: Instant,
    last: Instant,
    total_blocks: u64,
    blocks: u64,
    txs: u64,
    bytes: u64,
}

impl Progress {
    fn new(now: Instant) -> Self {
        Self {
            start: now,
            last: now,
            total_blocks: 0,
            blocks: 0,
            txs: 0,
            bytes: 0,
        }
    }

    /// Counts `block` at `height`, returns the progress line if it's time to log it
    fn block(
        &mut self,
        now: Instant,
        height: Height,
        tip: Height,
        block: &Block,
    ) -> Option<String> {
        self.total_blocks += 1;
        self.blocks += 1;
        self.txs += block.txdata.len() as u64;
        self.bytes += block.total_size() as u64;
        let elapsed = now.duration_since(self.last);
        if elapsed < PROGRESS_INTERVAL {
            return None;
        }
        let secs = elapsed.as_secs_f64();
        let blocks_per_sec = self.blocks as f64 / secs;
        let remaining = tip.saturating_sub(height) as f64 / blocks_per_sec;
        let line = format!(
            "indexing addresses: block {height}/{tip} ({:.1}%), last {}: {:.1} blocks/s, \
             {:.0} tx/s, {:.1} MB/s, ~{} left at this rate, {} elapsed",
            height as f64 * 100.0 / tip.max(1) as f64,
            format_duration(elapsed),
            blocks_per_sec,
            self.txs as f64 / secs,
            self.bytes as f64 / secs / 1_000_000.0,
            format_duration(Duration::from_secs_f64(remaining.min(1e9))),
            format_duration(now.duration_since(self.start)),
        );
        self.last = now;
        self.blocks = 0;
        self.txs = 0;
        self.bytes = 0;
        Some(line)
    }
}

/// `1d 2h`, `2h 3m`, `3m 4s` or `4s`: the two largest units are enough for a progress line
fn format_duration(duration: Duration) -> String {
    let secs = duration.as_secs();
    let (d, h, m, s) = (secs / 86_400, secs / 3_600 % 24, secs / 60 % 60, secs % 60);
    if d > 0 {
        format!("{d}d {h}h")
    } else if h > 0 {
        format!("{h}h {m}m")
    } else if m > 0 {
        format!("{m}m {s}s")
    } else {
        format!("{s}s")
    }
}

/// Downloads a block, and the outputs it spends if `with_spent`, retrying until it succeeds
async fn fetch_block(block_hash: BlockHash, with_spent: bool) -> (Block, Option<Vec<Vec<TxOut>>>) {
    loop {
        let spent = async {
            if with_spent {
                rpc::spenttxouts::call(block_hash).await.map(Some)
            } else {
                Ok(None)
            }
        };
        match tokio::try_join!(rpc::block::call(block_hash), spent) {
            Ok(result) => return result,
            Err(e) => {
                log::warn!("Cannot download block: {block_hash} {e}");
                tokio::time::sleep(Duration::from_secs(1)).await
            }
        }
    }
}

/// Aligns the balances with the active chain, after the initial indexing: drops the recent
/// blocks replaced by a reorg and pushes the missing ones.
pub(crate) async fn update_balance(db: &Database, shared_state: &SharedState) -> Result<(), Error> {
    if !db.is_synced() {
        return Ok(());
    }
    let (tip, recent) = db.balance_chain();
    if let Some(tip) = tip {
        if shared_state.height_to_hash(tip.height).await != Some(tip.block_hash) {
            log::error!(
                "reorg deeper than {REORG_DEPTH} blocks replaced {tip:?}, the address index \
                 balances need a reindex"
            );
            db.set_reindex_needed();
            return Ok(());
        }
    }
    for (height, block_hash) in recent {
        if shared_state.height_to_hash(height).await != Some(block_hash) {
            log::info!("reorg replaced block {height} {block_hash} in the balances");
            db.truncate_recent(height);
            break;
        }
    }
    loop {
        let height = db.next_balance_height();
        let block_hash = match shared_state.height_to_hash(height).await {
            Some(hash) if hash != BlockHash::all_zeros() => hash,
            _ => return Ok(()),
        };
        let block = rpc::block::call(block_hash).await?;
        let spent = rpc::spenttxouts::call(block_hash).await?;
        let deltas = db.block_deltas(&block, height, &spent)?;
        if !db.push_block(deltas)? {
            log::warn!("block {height} {block_hash} doesn't extend the balances");
            return Ok(());
        }
    }
}

#[cfg(test)]
mod test {
    #[test]
    fn latest_funding_heights() {
        use super::{Database, IndexBlockResult};
        use bitcoin::{hashes::Hash, BlockHash, ScriptBuf};
        use std::collections::BTreeSet;

        let dir = std::env::temp_dir().join(format!("fbbe-test-heights-{}", std::process::id()));
        let db = Database::new(&dir).unwrap();
        let script = ScriptBuf::from_bytes(vec![0x51]);
        let other = ScriptBuf::from_bytes(vec![0x52]);
        for (height, script) in [(5u32, &script), (7, &other), (9, &script), (12, &script)] {
            let index_res = IndexBlockResult {
                block_hash: BlockHash::hash(&height.to_be_bytes()),
                height,
                funding_sh: BTreeSet::from([db.hasher.hash(script)]),
                spending_sh: BTreeSet::new(),
            };
            db.write_hashes(index_res).unwrap();
        }
        assert_eq!(db.script_hash_latest_heights(&script, 2), vec![12, 9]);
        assert_eq!(db.script_hash_latest_heights(&script, 5), vec![12, 9, 5]);
        assert_eq!(db.script_hash_latest_heights(&other, 2), vec![7]);
        let unused = ScriptBuf::from_bytes(vec![0x53]);
        assert!(db.script_hash_latest_heights(&unused, 2).is_empty());
        assert!(!db.is_synced());

        drop(db);
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn test_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("fbbe-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// Block `height` of a test chain, every block hash derives from its height
    fn deltas(
        db: &super::Database,
        height: u32,
        changes: &[(&bitcoin::ScriptBuf, i64)],
    ) -> super::BlockDeltas {
        use bitcoin::{hashes::Hash, BlockHash};
        let hash = |h: u32| BlockHash::hash(&h.to_be_bytes());
        super::BlockDeltas {
            height,
            block_hash: hash(height),
            prev_blockhash: if height == 0 {
                BlockHash::all_zeros()
            } else {
                hash(height - 1)
            },
            deltas: changes
                .iter()
                .map(|(script, delta)| (db.hasher.hash(script), *delta))
                .collect(),
        }
    }

    #[test]
    fn balance_running_total() {
        use super::{Balance, Database, REORG_DEPTH};
        use bitcoin::{hashes::Hash, BlockHash, ScriptBuf};
        use std::sync::atomic::Ordering;

        let dir = test_dir("balance");
        let db = Database::new(&dir).unwrap();
        let a = ScriptBuf::from_bytes(vec![0x51]);
        let b = ScriptBuf::from_bytes(vec![0x52]);
        let unused = ScriptBuf::from_bytes(vec![0x53]);
        assert_eq!(db.balance(&a).unwrap(), Balance::IndexSyncing);
        db.synced.store(true, Ordering::Relaxed);

        assert!(db.push_block(deltas(&db, 0, &[(&a, 1_000)])).unwrap());
        assert!(db
            .push_block(deltas(&db, 1, &[(&a, -1_000), (&b, 900)]))
            .unwrap());
        assert!(db.push_block(deltas(&db, 2, &[(&a, 500)])).unwrap());
        // not counted twice, nor with a gap
        assert!(!db.push_block(deltas(&db, 2, &[(&a, 500)])).unwrap());
        assert!(!db.push_block(deltas(&db, 4, &[(&a, 500)])).unwrap());
        assert_eq!(db.balance(&a).unwrap(), Balance::Known(500));
        assert_eq!(db.balance(&b).unwrap(), Balance::Known(900));
        assert_eq!(db.balance(&unused).unwrap(), Balance::Known(0));
        assert_eq!(db.balance_chain().0, None, "all blocks still recent");

        for height in 3..=(REORG_DEPTH as u32 + 3) {
            assert!(db.push_block(deltas(&db, height, &[(&b, 1)])).unwrap());
        }
        let (tip, recent) = db.balance_chain();
        let tip = tip.unwrap();
        assert_eq!(tip.height, 3);
        assert_eq!(recent.len(), REORG_DEPTH);
        assert_eq!(recent[0].0, 4);
        assert_eq!(db.balance(&a).unwrap(), Balance::Known(500));
        assert_eq!(
            db.balance(&b).unwrap(),
            Balance::Known(900 + REORG_DEPTH as u64 + 1)
        );

        // a reorg of the recent blocks: replaced from height 7
        db.truncate_recent(7);
        assert_eq!(db.next_balance_height(), 7);
        let mut fork = deltas(&db, 7, &[(&a, 10)]);
        fork.block_hash = BlockHash::hash(b"fork");
        assert!(db.push_block(fork).unwrap());
        assert_eq!(db.balance(&a).unwrap(), Balance::Known(510));
        assert_eq!(db.balance(&b).unwrap(), Balance::Known(900 + 4));

        // after a restart, the tip and the script hash key are the same, the recent blocks are
        // pushed again
        let key = (db.hasher.k0, db.hasher.k1);
        drop(db);
        let db = Database::new(&dir).unwrap();
        db.synced.store(true, Ordering::Relaxed);
        assert_eq!(key, (db.hasher.k0, db.hasher.k1));
        assert_eq!(db.balance_chain(), (Some(tip), vec![]));
        assert_eq!(db.next_balance_height(), 4);
        assert_eq!(db.balance(&a).unwrap(), Balance::Known(500));
        assert_eq!(db.balance(&b).unwrap(), Balance::Known(900 + 1));

        db.set_reindex_needed();
        assert_eq!(db.balance(&a).unwrap(), Balance::ReindexNeeded);

        drop(db);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn block_deltas_from_block() {
        use bitcoin::{blockdata::constants::genesis_block, Network};

        let dir = test_dir("deltas");
        let db = super::Database::new(&dir).unwrap();
        let block = genesis_block(Network::Bitcoin);
        let deltas = db.block_deltas(&block, 0, &[vec![]]).unwrap();
        let script = &block.txdata[0].output[0].script_pubkey;
        assert_eq!(deltas.deltas.len(), 1);
        assert_eq!(deltas.deltas[&db.hasher.hash(script)], 5_000_000_000);
        assert!(db.block_deltas(&block, 0, &[]).is_err());
        assert!(db.block_deltas(&block, 0, &[vec![], vec![]]).is_err());

        drop(db);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn previous_format_refused() {
        let dir = test_dir("old-format");
        {
            let mut opts = rocksdb::Options::default();
            opts.create_if_missing(true);
            opts.create_missing_column_families(true);
            rocksdb::DB::open_cf(&opts, &dir, ["BLOCK_HASH_CF", "FUNDING_CF"]).unwrap();
        }
        let err = super::Database::new(&dir).unwrap_err().to_string();
        assert!(
            err.contains("point --addr-index-path to an empty directory"),
            "{err}"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn progress_line() {
        use super::{format_duration, Progress, PROGRESS_INTERVAL};
        use bitcoin::{blockdata::constants::genesis_block, Network};
        use std::time::{Duration, Instant};

        let block = genesis_block(Network::Bitcoin);
        let start = Instant::now();
        let mut progress = Progress::new(start);
        for height in 0..119 {
            assert!(progress.block(start, height, 1_000, &block).is_none());
        }
        let line = progress
            .block(start + PROGRESS_INTERVAL, 119, 1_000, &block)
            .unwrap();
        assert_eq!(
            line,
            "indexing addresses: block 119/1000 (11.9%), last 1m 0s: 2.0 blocks/s, 2 tx/s, \
             0.0 MB/s, ~7m 20s left at this rate, 1m 0s elapsed"
        );
        // counters restart from the line just logged
        assert!(progress
            .block(start + PROGRESS_INTERVAL, 120, 1_000, &block)
            .is_none());
        assert_eq!(progress.total_blocks, 121);

        assert_eq!(format_duration(Duration::from_secs(4)), "4s");
        assert_eq!(
            format_duration(Duration::from_secs(3 * 3_600 + 61)),
            "3h 1m"
        );
        assert_eq!(
            format_duration(Duration::from_secs(2 * 86_400 + 7_200)),
            "2d 2h"
        );
    }

    #[test]
    fn delta_encoding() {
        use super::{decode_delta, encode_delta};
        for value in [
            0,
            1,
            -1,
            63,
            -64,
            64,
            127,
            2_100_000_000_000_000,
            i64::MAX,
            i64::MIN,
        ] {
            assert_eq!(decode_delta(&encode_delta(value)), Some(value), "{value}");
        }
        assert_eq!(encode_delta(0), vec![0]);
        assert_eq!(encode_delta(-1), vec![1]);
        assert_eq!(encode_delta(1), vec![2]);
        assert_eq!(encode_delta(64).len(), 2);
        assert_eq!(decode_delta(&[]), None);
        assert_eq!(decode_delta(&[0x80]), None, "unterminated");
        assert_eq!(decode_delta(&[0, 0]), None, "trailing bytes");
        assert_eq!(decode_delta(&[0xff; 11]), None, "too long");
    }

    #[test]
    fn siphash_reference() {
        // SipHash-2-4 reference vector: key 00..0f, empty input
        let key: [u8; 16] = core::array::from_fn(|i| i as u8);
        let hasher = super::ScriptHasher::new(key);
        assert_eq!(hasher.hash(bitcoin::Script::new()), 0x726fdb47dd0e0e31);
    }

    #[test]
    fn find_outpoints_with_value() {
        use crate::rpc::block::SerBlock;
        use bitcoin::{blockdata::constants::genesis_block, consensus::serialize, Network};

        let block = genesis_block(Network::Bitcoin);
        let script_pubkey = block.txdata[0].output[0].script_pubkey.clone();
        let found = super::find_outpoints_with_script_pubkey(
            &script_pubkey,
            SerBlock(serialize(&block)),
            0,
            10,
        );
        let txid = block.txdata[0].compute_txid();
        assert_eq!(
            found,
            (
                vec![(bitcoin::OutPoint { txid, vout: 0 }, 5_000_000_000)],
                1
            )
        );
    }

    #[test]
    fn find_outpoints_window() {
        use crate::rpc::block::SerBlock;
        use bitcoin::{
            absolute::LockTime, blockdata::constants::genesis_block, consensus::serialize,
            transaction::Version, Amount, Network, OutPoint, ScriptBuf, Transaction, TxIn, TxOut,
        };

        let script = ScriptBuf::from_bytes(vec![0x51]);
        let other = ScriptBuf::from_bytes(vec![0x52]);
        let tx = |n: u64, outputs: &[&ScriptBuf]| Transaction {
            version: Version::TWO,
            lock_time: LockTime::from_consensus(n as u32),
            input: vec![TxIn::default()],
            output: outputs
                .iter()
                .enumerate()
                .map(|(i, s)| TxOut {
                    value: Amount::from_sat(n * 100 + i as u64),
                    script_pubkey: (*s).clone(),
                })
                .collect(),
        };
        let mut block = genesis_block(Network::Regtest);
        // 25 outputs paying `script` in 5 txs, mixed with outputs paying `other`
        block.txdata = (0..5u64)
            .map(|n| tx(n, &[&script, &other, &script, &script, &script, &script]))
            .collect();
        let expected: Vec<(OutPoint, u64)> = block
            .txdata
            .iter()
            .flat_map(|tx| {
                let txid = tx.compute_txid();
                tx.output
                    .iter()
                    .enumerate()
                    .filter(|(_, o)| o.script_pubkey == script)
                    .map(move |(vout, o)| (OutPoint::new(txid, vout as u32), o.value.to_sat()))
            })
            .collect();
        assert_eq!(expected.len(), 25);
        let ser = || SerBlock(serialize(&block));
        let find =
            |skip, take| super::find_outpoints_with_script_pubkey(&script, ser(), skip, take);

        assert_eq!(find(0, 10), (expected[0..10].to_vec(), 25));
        assert_eq!(find(10, 10), (expected[10..20].to_vec(), 25));
        assert_eq!(find(20, 10), (expected[20..25].to_vec(), 25));
        assert_eq!(find(3, 4), (expected[3..7].to_vec(), 25));
        assert_eq!(find(30, 10), (vec![], 25));
        assert_eq!(find(0, usize::MAX), (expected.clone(), 25));
        let found_other = super::find_outpoints_with_script_pubkey(&other, ser(), 0, 10);
        assert_eq!(found_other.0.len(), 5);
        assert_eq!(found_other.1, 5);
    }

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
