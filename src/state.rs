use std::{collections::HashMap, num::NonZeroUsize};

use bitcoin::{Block, BlockHash, Transaction, Txid};
use bitcoin_hashes::Hash;
use futures::prelude::*;
use lru::LruCache;
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

pub struct SharedState {
    // pub requests: AtomicUsize,
    // pub rpc_calls: AtomicUsize,
    pub chain_info: Mutex<ChainInfo>,
    pub txs: Mutex<LruCache<Txid, Transaction>>,
    pub tx_in_block: Mutex<LruCache<Txid, BlockHash>>,
    pub hash_to_height_time: Mutex<HashMap<BlockHash, HeightTime>>,
    pub height_to_hash: Mutex<Vec<BlockHash>>, // all zero if missing
    pub args: Arguments,
    pub mempool_info: Mutex<MempoolInfo>,
    pub mempool_fees: Mutex<MempoolFees>,
}

#[derive(Clone)]
pub struct MempoolFees {
    /// Highest fee tx in the mempool
    pub highest: Option<TxidWeightFee>,

    /// The fee of the last tx included in a block template of current mempool
    pub last_in_block: Option<TxidWeightFee>,

    /// The fee of the tx included in the middled of a block template of current mempool
    pub middle_in_block: Option<TxidWeightFee>,
}

impl SharedState {
    pub fn new(chain_info: ChainInfo, args: Arguments, mempool_info: MempoolInfo) -> Self {
        Self {
            // requests: AtomicUsize::new(0),
            // rpc_calls: AtomicUsize::new(0),
            chain_info: Mutex::new(chain_info),
            txs: Mutex::new(LruCache::new(
                NonZeroUsize::new(args.tx_cache_size).unwrap(),
            )),
            tx_in_block: Mutex::new(LruCache::new(NonZeroUsize::new(200_000).unwrap())),
            hash_to_height_time: Mutex::new(HashMap::new()),
            height_to_hash: Mutex::new(Vec::new()),
            args,
            mempool_info: Mutex::new(mempool_info),

            mempool_fees: Mutex::new(MempoolFees {
                highest: None,
                last_in_block: None,
                middle_in_block: None,
            }),
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
    ) -> Result<(Transaction, Option<BlockHash>), Error> {
        {
            let mut txs = self.txs.lock().await;
            if !needs_block_hash {
                if let Some(tx) = txs.get(&txid) {
                    log::trace!("tx hit");
                    return Ok((tx.clone(), None));
                }
            } else {
                let mut tx_in_block = self.tx_in_block.lock().await;
                match (txs.get(&txid), tx_in_block.get(&txid)) {
                    (Some(tx), Some(block_hash)) => {
                        log::trace!("tx hit");
                        return Ok((tx.clone(), Some(*block_hash)));
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
    ) -> Result<(Transaction, Option<BlockHash>), Error> {
        let (block_hash, tx) = rpc::tx::call_parse_json(txid, network()).await?;
        let mut txs = self.txs.lock().await;
        txs.put(txid, tx.clone());
        if let Some(block_hash) = block_hash {
            let mut tx_in_block = self.tx_in_block.lock().await;
            tx_in_block.put(txid, block_hash);
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
            .map(move |txid| rpc::tx::call_raw(txid))
            .buffer_unordered(self.args.fetch_parallelism)
            .collect()
            .await;

        let mut txs = self.txs.lock().await;

        for tx in got_txs {
            if let Ok(tx) = tx {
                txs.push(tx.txid(), tx);
            }
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

        for (txid, tx) in hash_tx {
            txs.put(txid, tx);
            tx_in_block.put(txid, block_hash);
        }

        if let Some(height) = height {
            let height_time = HeightTime { height, time };
            self.hash_to_height_time
                .lock()
                .await
                .insert(block_hash, height_time);
        }

        Ok(())
    }
}

pub(crate) fn reserve(height_to_hash: &mut MutexGuard<Vec<BlockHash>>, height: usize) {
    if height_to_hash.len() <= height {
        height_to_hash.resize(height + 1000, BlockHash::all_zeros());
    }
}
