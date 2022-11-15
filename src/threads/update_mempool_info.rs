use std::collections::HashMap;
use std::sync::Arc;

use crate::rpc;
use crate::state::SharedState;
use bitcoin::Txid;
use tokio::time::sleep;

pub async fn update_mempool(shared_state: Arc<SharedState>) {
    {
        let shared_state = shared_state.clone();
        tokio::spawn(async move {
            update_mempool_info(shared_state).await;
        });
    }
    update_mempool_details(shared_state).await;
}

async fn update_mempool_info(shared_state: Arc<SharedState>) {
    log::info!("Starting update_mempool_info");

    loop {
        if let Ok(mempool_info) = rpc::mempool::info().await {
            *shared_state.mempool_info.lock().await = mempool_info;
        }
        sleep(tokio::time::Duration::from_secs(2)).await;
    }
}

#[derive(Debug, Clone)]
struct WeightFee {
    weight: usize,
    fee: usize,
}

impl WeightFee {
    fn rate(&self) -> usize {
        (self.fee * 1_000_000) / self.weight
    }
}

async fn update_mempool_details(shared_state: Arc<SharedState>) {
    log::info!("Starting update_mempool_details");

    let mut cache: HashMap<Txid, WeightFee> = HashMap::new();
    let mut rates = vec![];

    loop {
        if let Ok(mempool) = rpc::mempool::content().await {
            cache.retain(|k, _v| mempool.contains(k)); // keep only current mempool elements
            log::trace!("mempool content returns {} txids", mempool.len(),);

            'outer: for txid in mempool {
                if cache.contains_key(&txid) {
                    continue;
                }
                if let Ok((tx, _)) = shared_state.tx(txid, false).await {
                    let mut sum_inputs = 0u64;
                    for input in tx.input.iter() {
                        if let Ok((prev_tx, _)) =
                            shared_state.tx(input.previous_output.txid, false).await
                        {
                            sum_inputs += prev_tx.output[input.previous_output.vout as usize].value;
                        } else {
                            continue 'outer;
                        }
                    }
                    let sum_outputs: u64 = tx.output.iter().map(|o| o.value).sum();
                    let fee = (sum_inputs - sum_outputs) as usize;
                    let weight = tx.weight();

                    cache.insert(txid, WeightFee { weight, fee });
                }
            }
        }

        rates.clear();
        rates.extend(cache.clone().into_iter());
        rates.sort_by_cached_key(|a| a.1.rate());

        let mut sum = 0;

        // TODO this doesn't take into account txs dependency
        let block_template: Vec<_> = rates
            .iter()
            .rev()
            .take_while(|i| {
                sum += i.1.weight;
                sum < 4_000_000
            })
            .collect();
        log::debug!("block template contains {}", block_template.len());

        let last_in_block = block_template.last().map(|t| t.0);
        let middle_in_block = block_template.get(block_template.len() / 2).map(|t| t.0);

        let mut mempool_fees = shared_state.mempool_fees.lock().await;

        mempool_fees.highest = rates.last().map(|t| t.0);
        mempool_fees.last_in_block = last_in_block;
        mempool_fees.middle_in_block = middle_in_block;

        drop(mempool_fees);

        sleep(tokio::time::Duration::from_secs(2)).await;

        log::trace!("mempool tx with fee: {}", rates.len());
    }
}
