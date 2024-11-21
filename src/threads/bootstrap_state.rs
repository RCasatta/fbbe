use crate::error::Error;
use crate::rpc::headers::HeightTime;
use crate::state::SharedState;
use crate::{network, rpc};
use bitcoin::blockdata::constants::genesis_block;
use bitcoin::hashes::Hash;
use bitcoin::BlockHash;
use std::collections::HashMap;
use std::sync::Arc;

const HEADERS_PER_REQUEST: usize = 101;

pub(crate) async fn bootstrap_state_infallible(shared_state: Arc<SharedState>) {
    if let Err(e) = bootstrap_state(shared_state).await {
        log::error!("{:?}", e);
    }
}

pub async fn bootstrap_state(shared_state: Arc<SharedState>) -> Result<(), Error> {
    let geneis_hash = genesis_block(network()).header.block_hash();
    let mut hash = geneis_hash;
    let mut height = 0;
    let mut hash_to_height_time = HashMap::new();

    for i in (0usize..).step_by(HEADERS_PER_REQUEST - 1) {
        let headers = rpc::headers::call_many(hash, HEADERS_PER_REQUEST as u32).await?;
        {
            for (j, header) in headers.iter().enumerate() {
                hash = header.block_hash();
                height = (i + j) as u32;
                let time = header.time;

                hash_to_height_time.insert(hash, HeightTime { height, time });
            }
            if headers.len() != HEADERS_PER_REQUEST {
                break;
            }
        }
    }

    for (k, v) in hash_to_height_time.iter() {
        shared_state.add_height_hash(v.height, *k).await;
    }

    shared_state
        .bootstrap_hash_to_height_time(hash_to_height_time)
        .await;

    let mut current = shared_state.chain_info.lock().await.best_block_hash;
    let mut count = 0;
    loop {
        let block = rpc::block::call(current).await?;
        current = block.header.prev_blockhash;
        shared_state.update_cache(&block, None).await?;
        count += 1;
        let cache = shared_state.txs.lock().await;
        if cache.full() {
            log::info!(
                "tx cache full of {} elements with {count} blocks",
                cache.len()
            );
            break;
        }
        if current == BlockHash::all_zeros() {
            log::info!("reached genesis in bootstraping state, breaking");
            break;
        }
    }

    log::info!("bootstrap ending, headers ending at {}", height);

    Ok(())
}
