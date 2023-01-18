use crate::error::Error;
use crate::rpc::headers::HeightTime;
use crate::state::{reserve, SharedState};
use crate::{network, rpc};
use bitcoin::BlockHash;
use bitcoin::blockdata::constants::genesis_block;
use bitcoin_hashes::Hash;
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
    for i in (0usize..).step_by(HEADERS_PER_REQUEST - 1) {
        let headers = rpc::headers::call_many(hash, HEADERS_PER_REQUEST as u32).await?;
        {
            let mut hash_to_height_time = shared_state.hash_to_height_time.lock().await;
            let mut height_to_hash = shared_state.height_to_hash.lock().await;
            for (j, header) in headers.iter().enumerate() {
                hash = header.block_hash();
                height = (i + j) as u32;
                let time = header.time;

                hash_to_height_time.insert(hash, HeightTime { height, time });

                reserve(&mut height_to_hash, height as usize);
                height_to_hash[height as usize] = hash;
            }
            if headers.len() != HEADERS_PER_REQUEST {
                log::info!("headers ending at {}", height);
                break;
            }
        }
    }
    let mut current = shared_state.chain_info.lock().await.best_block_hash;
    let mut count = 0;
    loop {
        let block = rpc::block::call_raw(current).await?;
        current = block.header.prev_blockhash;
        shared_state.update_cache(block, None).await?;
        count += 1;
        let len = shared_state.txs.lock().await.len();
        if len > shared_state.args.tx_cache_size / 2 {
            log::info!("tx cache full at 50% {len} with {count} blocks");
            break;
        }
        if current == BlockHash::all_zeros() {
            log::info!("reached genesis in bootstraping state, breaking");
            break;
        }
    }

    Ok(())
}
