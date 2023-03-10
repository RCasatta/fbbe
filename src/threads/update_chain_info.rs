use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use crate::error::Error;
use crate::rpc;
use crate::rpc::chaininfo::ChainInfo;
use crate::state::SharedState;
use bitcoin::BlockHash;
use bitcoin_hashes::Hash;
use tokio::time::sleep;

pub(crate) async fn update_chain_info_infallible(
    shared_state: Arc<SharedState>,
    initial_chain_info: ChainInfo,
) {
    if let Err(e) = update_chain_info(shared_state, initial_chain_info).await {
        log::error!("{:?}", e);
    }
}

async fn update_chain_info(
    shared_state: Arc<SharedState>,
    initial_chain_info: ChainInfo,
) -> Result<(), Error> {
    log::info!("Starting update_chain_info");

    let mut current = initial_chain_info;
    loop {
        update_blocks_in_last_hour(&shared_state, current.blocks as usize).await;

        sleep(tokio::time::Duration::from_secs(2)).await;

        match rpc::chaininfo::call().await {
            Ok(last_tip) => {
                if last_tip != current {
                    // this hit even if height is the same but block hash different -> reorg
                    log::info!("New tip! {:?}", last_tip);

                    let mut last_height = last_tip.blocks;
                    let mut last_block_hash = last_tip.best_block_hash;

                    loop {
                        log::info!("asking {last_block_hash}");
                        let last_block = rpc::block::call_raw(last_block_hash).await?;
                        let prev_blockhash = last_block.header.prev_blockhash;

                        shared_state
                            .update_cache(last_block, Some(last_height))
                            .await?;

                        match shared_state
                            .hash_to_height_time
                            .lock()
                            .await
                            .get(&prev_blockhash)
                        {
                            Some(height_time) if height_time.height == last_height - 1 => {
                                log::debug!("previous block has correct height, breaking");
                                break;
                            }
                            _ => {
                                log::info!(
                                    "cache missing or reorg longer than 1 happened, going back"
                                );
                                last_block_hash = prev_blockhash;
                                last_height -= 1;
                            }
                        }
                    }

                    current = last_tip.clone();
                    *shared_state.chain_info.lock().await = last_tip;
                }
            }
            Err(e) => {
                log::warn!("{:?}", e);
            }
        }
    }
}

async fn update_blocks_in_last_hour(shared_state: &Arc<SharedState>, last_tip_height: usize) {
    let mut count = 0;
    let height_to_hash = shared_state.height_to_hash.lock().await;
    for i in (0..last_tip_height).rev() {
        let hash = height_to_hash[i];

        if hash != BlockHash::all_zeros() {
            if let Ok(ht) = shared_state.height_time(hash).await {
                if ht.since_now() > Duration::from_secs(60 * 60) {
                    break;
                } else {
                    count += 1;
                }
            }
        }
    }
    shared_state
        .blocks_in_last_hour
        .store(count, Ordering::Relaxed);
}
