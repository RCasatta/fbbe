use std::collections::HashMap;
use std::ops::ControlFlow;
use std::time::Instant;

use bitcoin::consensus::Encodable;
use bitcoin::hashes::Hash;
use bitcoin::OutPoint;
use bitcoin::{Block, BlockHash, Transaction, Txid, Weight};
use bitcoin_slices::Parse;
use bitcoin_slices::{bsl, SliceCache, Visit, Visitor};
use futures::prelude::*;
use fxhash::FxHashMap;
use fxhash::FxHashSet;
use lru::LruCache;
use prometheus::Registry;
use tokio::sync::{Mutex, MutexGuard};

use crate::cache_counter;
use crate::rpc::block::SerBlock;
use crate::{
    error::Error,
    network,
    rpc::{self, chaininfo::ChainInfo, headers::HeightTime, mempool::MempoolInfo},
    threads::update_mempool_info::TxidWeightFee,
    Arguments,
};

// pub const VERSION: u32 = 0;

// testnet 10_000 txs, but 2M headers -> 64Mb only height_to_hash, 80Mb of hash_to_height_time | 250Mb
// signet 10_000 txs | 25Mb

/// Contains a serialized transaction.
/// `Transaction` is not used directly because it keeps long-lived small allocations alive in the
/// cache.
#[derive(Debug, Clone)]
pub struct SerTx(pub Vec<u8>);

impl AsRef<[u8]> for SerTx {
    fn as_ref(&self) -> &[u8] {
        &self.0[..]
    }
}

#[derive(Eq, Hash, PartialEq)]
pub struct TruncTxid([u8; 8]);

impl From<Txid> for TruncTxid {
    fn from(value: Txid) -> Self {
        TruncTxid(
            value.to_byte_array()[..8]
                .try_into()
                .expect("slice right length"),
        )
    }
}

impl From<&Txid> for TruncTxid {
    fn from(value: &Txid) -> Self {
        TruncTxid(
            value.as_byte_array()[..8]
                .try_into()
                .expect("slice right length"),
        )
    }
}

pub struct SharedState {
    // pub requests: AtomicUsize,
    // pub rpc_calls: AtomicUsize,
    pub chain_info: Mutex<ChainInfo>,

    /// By default 100MB of cached transactions, `Txid -> Transaction`
    pub txs: Mutex<SliceCache<Txid>>,

    /// Up to 1M elements
    /// TODO truncate key to 8 bytes or so, use height as key, keep a lot more
    pub tx_in_block: Mutex<LruCache<TruncTxid, BlockHash>>,

    hash_to_height_time: Mutex<FxHashMap<BlockHash, HeightTime>>,

    /// mainnet 800k -> at least 800_000 * 32 B = 25.6 MB
    pub height_to_hash: Mutex<Vec<BlockHash>>, // all zero if missing

    pub args: Arguments,
    pub mempool_info: Mutex<MempoolInfo>,
    pub mempool_fees: Mutex<BlockTemplate>,
    pub minutes_since_block: Mutex<Option<String>>,

    // Added when found tx in mempool, removed when not in mempool
    // for each inputs in the mempool the SpendPoint and the relatvie spent OutPoint
    // if the mempool has 100k with an average of 1.5 inputs, we have 150k*(36+36) = 10MB
    pub mempool_spending: Mutex<FxHashMap<OutPoint, SpendPoint>>,

    /// A note on known transactions
    pub known_txs: HashMap<Txid, String>,
}

#[derive(Debug, Clone)]
pub struct SpendPoint {
    txid: Txid,
    vin: u32,
}

impl SpendPoint {
    pub fn new(txid: Txid, vin: u32) -> Self {
        Self { txid, vin }
    }

    pub(crate) fn txid(&self) -> &Txid {
        &self.txid
    }

    pub(crate) fn vin(&self) -> u32 {
        self.vin
    }
}

#[derive(Clone)]
pub struct BlockTemplate {
    /// Highest fee tx in the mempool
    pub highest: Option<TxidWeightFee>,

    /// The fee of the last tx included in a block template of current mempool
    pub last_in_block: Option<TxidWeightFee>,

    /// The fee of the tx included in the middled of a block template of current mempool
    pub middle_in_block: Option<TxidWeightFee>,

    /// Number of transactions in the block template
    pub transactions: Option<usize>,

    /// Transactions in the mempool
    pub mempool: FxHashSet<Txid>,
}

impl SharedState {
    pub fn new(
        chain_info: ChainInfo,
        args: Arguments,
        mempool_info: MempoolInfo,
        known_txs: HashMap<Txid, String>,
        registry: &Registry,
    ) -> Self {
        let txs = SliceCache::new(args.tx_cache_byte_size());
        txs.register_metric(registry).unwrap(); // TODO
        Self {
            // requests: AtomicUsize::new(0),
            // rpc_calls: AtomicUsize::new(0),
            chain_info: Mutex::new(chain_info),
            txs: Mutex::new(txs),
            tx_in_block: Mutex::new(LruCache::new(args.txid_blockhash_len().try_into().unwrap())), //TODO
            hash_to_height_time: Mutex::new(FxHashMap::default()),
            height_to_hash: Mutex::new(Vec::new()),
            args,
            mempool_info: Mutex::new(mempool_info),

            mempool_fees: Mutex::new(BlockTemplate {
                highest: None,
                last_in_block: None,
                middle_in_block: None,
                transactions: None,
                mempool: FxHashSet::default(),
            }),
            minutes_since_block: Mutex::new(None),
            mempool_spending: Mutex::new(FxHashMap::default()),
            known_txs,
        }
    }

    pub async fn blocks_from_heights(
        &self,
        heights: &[u32],
    ) -> Result<Vec<(BlockHash, SerBlock)>, Error> {
        let mut res = vec![];
        // TODO cache some blocks
        for h in heights {
            if let Some(block_hash) = self.height_to_hash.lock().await.get(*h as usize) {
                let block = rpc::block::call_raw(*block_hash).await?; // TODO use raw block SerBlock
                res.push((*block_hash, block))
            }
        }
        Ok(res)
    }

    pub async fn bootstrap_hash_to_height_time(&self, map: HashMap<BlockHash, HeightTime>) {
        self.hash_to_height_time.lock().await.extend(map);
    }

    pub async fn height_time(&self, block_hash: BlockHash) -> Result<HeightTime, Error> {
        let timestamp = self
            .hash_to_height_time
            .lock()
            .await
            .get(&block_hash)
            .cloned();

        cache_counter("height-time", timestamp.is_some());

        if let Some(height_time) = timestamp {
            Ok(height_time)
        } else {
            let header = rpc::headers::call_one(block_hash).await?;
            self.hash_to_height_time
                .lock()
                .await
                .insert(block_hash, header.height_time);

            let height = header.height() as usize;
            let mut height_to_hash = self.height_to_hash.lock().await;
            reserve(&mut height_to_hash, height);
            height_to_hash[height] = block_hash;

            Ok(header.height_time)
        }
    }

    pub async fn hash(&self, height: u32) -> Result<BlockHash, Error> {
        let height = height as usize;
        let mut height_to_hash = self.height_to_hash.lock().await;
        reserve(&mut height_to_hash, height);
        if height_to_hash[height] != BlockHash::all_zeros() {
            log::trace!("height hit");
            Ok(height_to_hash[height])
        } else {
            log::debug!("height miss");
            let r = rpc::blockhashbyheight::call(height).await?;
            height_to_hash[height] = r.block_hash;
            Ok(r.block_hash)
        }
    }

    pub async fn tx(
        &self,
        txid: Txid,
        needs_block_hash: bool,
    ) -> Result<(SerTx, Option<BlockHash>), Error> {
        {
            let tx = self
                .txs
                .lock()
                .await
                .get(&txid)
                .map(|tx| SerTx(tx.to_vec()));

            if !needs_block_hash {
                match tx {
                    Some(tx) => Ok((tx, None)),
                    None => {
                        let tx = rpc::tx::call_raw(txid).await?;
                        let _ = self.txs.lock().await.insert(txid, &tx);
                        Ok((SerTx(tx), None))
                    }
                }
            } else {
                let block_hash = self.tx_in_block.lock().await.get(&txid.into()).cloned();
                cache_counter("txid-block_hash", block_hash.is_some());

                match (tx, block_hash) {
                    (Some(tx), Some(block_hash)) => Ok((tx, Some(block_hash))),
                    (Some(tx), None) => {
                        // getting only the block hash
                        let block_hash = rpc::tx::call_json_only_hash(txid).await?;
                        if let Some(block_hash) = block_hash {
                            self.tx_in_block.lock().await.push(txid.into(), block_hash);
                        }
                        Ok((tx, block_hash))
                    }
                    (None, Some(block_hash)) => {
                        // getting only the tx
                        let tx = rpc::tx::call_raw(txid).await?;
                        let _ = self.txs.lock().await.insert(txid, &tx);
                        Ok((SerTx(tx), Some(block_hash)))
                    }
                    (None, None) => self.tx_fetch_and_cache(txid, needs_block_hash).await,
                }
            }
        }
    }

    pub async fn tx_fetch_and_cache(
        &self,
        txid: Txid,
        needs_block_hash: bool,
    ) -> Result<(SerTx, Option<BlockHash>), Error> {
        let (block_hash, tx) = if needs_block_hash {
            rpc::tx::call_parse_json(txid, network()).await?
        } else {
            (None::<BlockHash>, SerTx(rpc::tx::call_raw(txid).await?))
        };
        let mut txs = self.txs.lock().await;

        let _ = txs.insert(txid, &tx.0);

        if let Some(block_hash) = block_hash {
            let mut tx_in_block = self.tx_in_block.lock().await;
            let _ = tx_in_block.put(txid.into(), block_hash);
        }
        Ok((tx, block_hash))
    }

    pub async fn preload_prevouts(&self, txid: Txid, tx: &Transaction) {
        self.preload_prevouts_inner(txid, tx.input.iter().map(|i| &i.previous_output))
            .await;
    }

    pub async fn preload_prevouts_inner(
        &self,
        txid: Txid,
        tx_ins: impl Iterator<Item = &OutPoint>,
    ) {
        let mut count = 0;
        let needed: Vec<_> = {
            let txs = self.txs.lock().await;

            tx_ins
                .map(|o| o.txid)
                .inspect(|_| count += 1)
                .filter(|t| !txs.contains(t) && t != &Txid::all_zeros())
                .collect()
        };

        let needed_len = needed.len();
        let start = Instant::now();

        let got_txs: Vec<_> = stream::iter(needed)
            .map(rpc::tx::call_raw)
            .buffer_unordered(self.args.fetch_parallelism)
            .collect()
            .await;

        let mut txs = self.txs.lock().await;

        for tx in got_txs.into_iter().flatten() {
            if let Ok(res) = bsl::Transaction::parse(&tx) {
                let tx = res.parsed();
                let txid = Txid::from_byte_array(tx.txid_sha2().into());
                let _ = txs.insert(txid, tx);
            }
        }

        if needed_len > 100 {
            log::info!(
                "needed {} prevouts (out of {}) for {} loaded in {}ms",
                needed_len,
                count,
                txid,
                start.elapsed().as_millis()
            );
        }
    }

    pub async fn update_cache(&self, block: &Block, height: Option<u32>) -> Result<(), Error> {
        let block_hash = block.block_hash();
        let time = block.header.time;
        let hash_tx: Vec<_> = block
            .txdata
            .iter()
            .map(|tx| (tx.compute_txid(), tx))
            .collect();

        let mut txs = self.txs.lock().await;
        let mut tx_in_block = self.tx_in_block.lock().await;
        let mut buffer = vec![];

        for (txid, tx) in hash_tx {
            buffer.clear();
            tx.consensus_encode(&mut buffer).expect("vecs don't error");
            let _ = txs.insert(txid, &buffer);
            let _ = tx_in_block.put(txid.into(), block_hash);
        }

        if let Some(height) = height {
            let height_time = HeightTime { height, time };
            self.hash_to_height_time
                .lock()
                .await
                .insert(block_hash, height_time);

            let mut height_to_hash = self.height_to_hash.lock().await;
            reserve(&mut height_to_hash, height as usize);
            height_to_hash[height as usize] = block_hash;
        }

        Ok(())
    }
}

pub(crate) fn reserve(height_to_hash: &mut MutexGuard<Vec<BlockHash>>, height: usize) {
    if height_to_hash.len() <= height {
        height_to_hash.resize(height + 1000, BlockHash::all_zeros());
    }
}

pub struct OutPointsAndSum {
    pub prevouts: Vec<bitcoin::OutPoint>,
    pub sum: u64,
    pub weight: Weight,
}
impl Visitor for OutPointsAndSum {
    fn visit_transaction(&mut self, tx: &bsl::Transaction) -> ControlFlow<()> {
        self.weight = Weight::from_wu(tx.weight());
        ControlFlow::Continue(())
    }
    fn visit_tx_out(&mut self, _vout: usize, tx_out: &bsl::TxOut) -> ControlFlow<()> {
        self.sum += tx_out.value();
        ControlFlow::Continue(())
    }
    fn visit_tx_ins(&mut self, total_inputs: usize) {
        self.prevouts.reserve(total_inputs);
    }
    fn visit_tx_in(&mut self, _vin: usize, tx_in: &bsl::TxIn) -> ControlFlow<()> {
        self.prevouts.push(tx_in.prevout().into()); // TODO don't parse the script if it isn't of interest?
        ControlFlow::Continue(())
    }
}
pub fn outpoints_and_sum(tx_bytes: &[u8]) -> Result<OutPointsAndSum, bitcoin_slices::Error> {
    let mut visitor = OutPointsAndSum {
        prevouts: Vec::new(),
        sum: 0,
        weight: Weight::ZERO,
    };
    bsl::Transaction::visit(tx_bytes, &mut visitor)?;
    Ok(visitor)
}

pub fn tx_output(
    tx_bytes: &[u8],
    vout: u32,
    needs_script: bool,
) -> Result<bitcoin::TxOut, bitcoin_slices::Error> {
    struct TxOutput {
        vout: u32,
        tx_out: bitcoin::TxOut,
        needs_script: bool,
    }
    impl Visitor for TxOutput {
        fn visit_tx_out(&mut self, vout: usize, tx_out: &bsl::TxOut) -> ControlFlow<()> {
            if self.vout == vout as u32 {
                if self.needs_script {
                    self.tx_out = tx_out.into();
                } else {
                    self.tx_out = bitcoin::TxOut {
                        script_pubkey: bitcoin::ScriptBuf::new(),
                        value: bitcoin::Amount::from_sat(tx_out.value()),
                    };
                }
                return ControlFlow::Break(());
            }
            ControlFlow::Continue(())
        }
    }
    let mut visitor = TxOutput {
        vout,
        tx_out: bitcoin::TxOut::NULL,
        needs_script,
    };
    match bsl::Transaction::visit(tx_bytes, &mut visitor) {
        Ok(_) | Err(bitcoin_slices::Error::VisitBreak) => Ok(visitor.tx_out),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod test {

    use crate::state::outpoints_and_sum;

    #[test]
    fn test_prevouts() {
        const SOME_TX: &str = "0100000001a15d57094aa7a21a28cb20b59aab8fc7d1149a3bdbcddba9c622e4f5f6a99ece010000006c493046022100f93bb0e7d8db7bd46e40132d1f8242026e045f03a0efe71bbb8e3f475e970d790221009337cd7f1f929f00cc6ff01f03729b069a7c21b59b1736ddfee5db5946c5da8c0121033b9b137ee87d5a812d6f506efdd37f0affa7ffc310711c06c7f3e097c9447c52ffffffff0100e1f505000000001976a9140389035a9225b3839e2bbf32d826a1e222031fd888ac00000000";
        let bytes = hex::decode(SOME_TX).unwrap();
        let res = outpoints_and_sum(&bytes[..]).unwrap();
        assert_eq!(res.sum, 100000000);
        assert_eq!(res.prevouts.len(), 1);
    }
}
