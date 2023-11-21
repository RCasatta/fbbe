use crate::error::Error;
use crate::rpc::headers::HeightTime;
use crate::state::{reserve, SharedState};
use crate::{network, rpc};
use bitcoin::blockdata::constants::genesis_block;
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
                break;
            }
        }
    }
    let current = shared_state.chain_info.lock().await.best_block_hash;
    let block = rpc::block::call(current).await?;
    shared_state.update_cache(&block, None).await?;

    log::info!("bootstrap ending, headers ending at {}", height);

    Ok(())
}
