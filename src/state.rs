use std::collections::HashMap;
use std::collections::HashSet;
use std::ops::ControlFlow;

use bitcoin::consensus::Encodable;
use bitcoin::hashes::Hash;
use bitcoin::{Block, BlockHash, Transaction, Txid, Weight};
use bitcoin_slices::{bsl, SliceCache, Visit, Visitor};
use futures::prelude::*;
use tokio::sync::{Mutex, MutexGuard};

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

pub struct SharedState {
    // pub requests: AtomicUsize,
    // pub rpc_calls: AtomicUsize,
    pub chain_info: Mutex<ChainInfo>,

    /// By default 100MB of cached transactions, `Txid -> Transaction`
    pub txs: Mutex<SliceCache<Txid>>,

    /// 20MB of `Txid -> BlockHash` ~300k elements
    pub tx_in_block: Mutex<SliceCache<Txid>>,

    pub hash_to_height_time: Mutex<HashMap<BlockHash, HeightTime>>,

    /// mainnet 800k -> at least 800_000 * 32 B = 25.6 MB
    pub height_to_hash: Mutex<Vec<BlockHash>>, // all zero if missing

    pub args: Arguments,
    pub mempool_info: Mutex<MempoolInfo>,
    pub mempool_fees: Mutex<BlockTemplate>,
    pub minutes_since_block: Mutex<Option<String>>,
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
    pub mempool: HashSet<Txid>,
}

impl SharedState {
    pub fn new(chain_info: ChainInfo, args: Arguments, mempool_info: MempoolInfo) -> Self {
        Self {
            // requests: AtomicUsize::new(0),
            // rpc_calls: AtomicUsize::new(0),
            chain_info: Mutex::new(chain_info),
            txs: Mutex::new(SliceCache::new(args.tx_cache_byte_size)),
            tx_in_block: Mutex::new(SliceCache::new(args.tx_block_byte_size)),
            hash_to_height_time: Mutex::new(HashMap::new()),
            height_to_hash: Mutex::new(Vec::new()),
            args,
            mempool_info: Mutex::new(mempool_info),

            mempool_fees: Mutex::new(BlockTemplate {
                highest: None,
                last_in_block: None,
                middle_in_block: None,
                transactions: None,
                mempool: HashSet::new(),
            }),
            minutes_since_block: Mutex::new(None),
        }
    }

    pub async fn height_time(&self, block_hash: BlockHash) -> Result<HeightTime, Error> {
        let mut hash_to_timestamp = self.hash_to_height_time.lock().await;
        if let Some(height_time) = hash_to_timestamp.get(&block_hash) {
            log::trace!("timestamp hit");
            Ok(*height_time)
        } else {
            log::debug!("timestamp miss");
            // let _ = self.rpc_calls.fetch_add(1, Ordering::Relaxed);
            let header = rpc::headers::call_one(block_hash).await?;
            hash_to_timestamp.insert(block_hash, header.height_time);
            drop(hash_to_timestamp);

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
            let txs = self.txs.lock().await;
            if !needs_block_hash {
                if let Some(tx) = txs.get(&txid) {
                    log::trace!("tx hit");
                    return Ok((SerTx(tx.to_vec()), None));
                }
            } else {
                let tx_in_block = self.tx_in_block.lock().await;
                match (txs.get(&txid), tx_in_block.get(&txid)) {
                    (Some(tx), Some(block_hash)) => {
                        let block_hash = BlockHash::from_slice(block_hash)
                            .expect("cache contains only block hashes");
                        log::trace!("tx hit");
                        return Ok((SerTx(tx.to_vec()), Some(block_hash)));
                    }
                    (Some(_), None) => log::debug!("tx miss, missing block"),
                    (None, Some(_)) => log::debug!("tx miss, missing tx"),
                    (None, None) => log::debug!("tx miss, missing tx and block"),
                }
            }
        }
        self.tx_fetch_and_cache(txid).await
    }

    pub async fn tx_fetch_and_cache(
        &self,
        txid: Txid,
    ) -> Result<(SerTx, Option<BlockHash>), Error> {
        let (block_hash, tx) = rpc::tx::call_parse_json(txid, network()).await?;
        let mut txs = self.txs.lock().await;

        let _ = txs.insert(txid, &tx.0);

        if let Some(block_hash) = block_hash {
            let mut tx_in_block = self.tx_in_block.lock().await;
            let _ = tx_in_block.insert(txid, block_hash.as_byte_array());
        }
        Ok((tx, block_hash))
    }

    pub async fn preload_prevouts(&self, tx: &Transaction) {
        let needed: Vec<_> = {
            let txs = self.txs.lock().await;

            tx.input
                .iter()
                .map(|i| i.previous_output.txid)
                .filter(|t| !txs.contains(t) && t != &Txid::all_zeros())
                .collect()
        };

        let needed_len = needed.len();
        if needed_len > 30 {
            log::info!("needed {} prevouts for {}", needed_len, tx.txid());
        }

        let got_txs: Vec<_> = stream::iter(needed)
            .map(rpc::tx::call_raw)
            .buffer_unordered(self.args.fetch_parallelism)
            .collect()
            .await;

        let mut txs = self.txs.lock().await;

        let mut buffer = vec![];
        for tx in got_txs.into_iter().flatten() {
            buffer.clear();
            tx.consensus_encode(&mut buffer).expect("vecs don't error");
            let _ = txs.insert(tx.txid(), &buffer);
        }

        if needed_len > 30 {
            log::info!("needed {} prevouts for {} loaded", needed_len, tx.txid());
        }
    }

    pub async fn update_cache(&self, block: Block, height: Option<u32>) -> Result<(), Error> {
        let block_hash = block.block_hash();
        let time = block.header.time;
        let hash_tx: Vec<_> = block.txdata.into_iter().map(|tx| (tx.txid(), tx)).collect();

        let mut txs = self.txs.lock().await;
        let mut tx_in_block = self.tx_in_block.lock().await;
        let mut buffer = vec![];

        for (txid, tx) in hash_tx {
            buffer.clear();
            tx.consensus_encode(&mut buffer).expect("vecs don't error");
            let _ = txs.insert(txid, &buffer);
            let _ = tx_in_block.insert(txid, block_hash.as_byte_array());
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
    struct Res {
        vout: u32,
        tx_out: bitcoin::TxOut,
        needs_script: bool,
    }
    impl Visitor for Res {
        fn visit_tx_out(&mut self, vout: usize, tx_out: &bsl::TxOut) -> ControlFlow<()> {
            if self.vout == vout as u32 {
                if self.needs_script {
                    self.tx_out = tx_out.into();
                } else {
                    self.tx_out = bitcoin::TxOut {
                        script_pubkey: bitcoin::ScriptBuf::new(),
                        value: tx_out.value(),
                    };
                }
                return ControlFlow::Break(());
            }
            ControlFlow::Continue(())
        }
    }
    let mut visitor = Res {
        vout,
        tx_out: bitcoin::TxOut::default(),
        needs_script,
    };
    match bsl::Transaction::visit(tx_bytes, &mut visitor) {
        Ok(_) | Err(bitcoin_slices::Error::VisitBreak) => Ok(visitor.tx_out),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod test {
    use bitcoin::hashes::hex::FromHex;

    use crate::state::outpoints_and_sum;

    #[test]
    fn test_prevouts() {
        const SOME_TX: &str = "0100000001a15d57094aa7a21a28cb20b59aab8fc7d1149a3bdbcddba9c622e4f5f6a99ece010000006c493046022100f93bb0e7d8db7bd46e40132d1f8242026e045f03a0efe71bbb8e3f475e970d790221009337cd7f1f929f00cc6ff01f03729b069a7c21b59b1736ddfee5db5946c5da8c0121033b9b137ee87d5a812d6f506efdd37f0affa7ffc310711c06c7f3e097c9447c52ffffffff0100e1f505000000001976a9140389035a9225b3839e2bbf32d826a1e222031fd888ac00000000";
        let bytes = Vec::<u8>::from_hex(SOME_TX).unwrap();
        let res = outpoints_and_sum(&bytes[..]).unwrap();
        assert_eq!(res.sum, 100000000);
        assert_eq!(res.prevouts.len(), 1);
    }
}
