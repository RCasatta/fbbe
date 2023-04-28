use std::collections::HashMap;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::rpc;
use crate::state::SharedState;
use bitcoin::Txid;
use maud::{html, Render};
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
pub struct WeightFee {
    /// The weight of the tx in vbytes
    pub weight: usize, // TODO: change to Weight

    /// The absolute fee in satoshi
    pub fee: usize,
}

#[derive(Debug, Clone)]
pub struct TxidWeightFee {
    pub wf: WeightFee,
    pub txid: Txid,
}

impl Render for WeightFee {
    fn render(&self) -> maud::Markup {
        // em data-tooltip=(rate_sat_vb) style="font-style: normal" { (rate_btc_kvb)
        let btc_over_kvb = format!("{:.8}", self.rate_btc_over_kvb());
        let sat_over_vb = self.sat_over_vb_str();

        html! { em data-tooltip=(sat_over_vb) style="font-style: normal" { (btc_over_kvb) } }
    }
}

impl WeightFee {
    fn rate(&self) -> usize {
        (self.fee * 1_000_000) / self.weight
    }

    fn rate_btc_over_kvb(&self) -> f64 {
        (self.fee as f64 / 100_000_000.0) / (self.weight as f64 / 4_000.0)
    }
    fn rate_sat_over_vb(&self) -> f64 {
        (self.fee as f64) / (self.weight as f64 / 4.0)
    }
    pub fn sat_over_vb_str(&self) -> String {
        format!("{:.1} sat/vB", self.rate_sat_over_vb())
    }
}

async fn update_mempool_details(shared_state: Arc<SharedState>) {
    log::info!("Starting update_mempool_details");

    let mut cache: HashMap<Txid, WeightFee> = HashMap::new();
    let mut rates: Vec<TxidWeightFee> = vec![];

    loop {
        if let Ok(mempool) = rpc::mempool::content().await {
            cache.retain(|k, _v| mempool.contains(k)); // keep only current mempool elements
            log::trace!("mempool content returns {} txids", mempool.len());

            let start = Instant::now();
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
                    let weight = tx.weight().to_wu() as usize;

                    cache.insert(txid, WeightFee { weight, fee });

                    if start.elapsed() > Duration::from_secs(60) {
                        log::info!(
                            "mempool info is taking more than a minute, breaking. Cache len: {}",
                            cache.len()
                        );
                        break;
                    }
                }
            }
        }

        rates.clear();
        rates.extend(
            cache
                .clone()
                .into_iter()
                .map(|(txid, wf)| TxidWeightFee { wf, txid }),
        );
        rates.sort_by_cached_key(|a| a.wf.rate());

        let mut sum = 0;

        // TODO this doesn't take into account txs dependency
        let block_template: Vec<_> = rates
            .iter()
            .rev()
            .take_while(|i| {
                sum += i.wf.weight;
                sum < 4_000_000
            })
            .collect();
        log::debug!("block template contains {}", block_template.len());

        let last_in_block = block_template.last().cloned();
        let middle_in_block = block_template.get(block_template.len() / 2).cloned();

        let mut mempool_fees = shared_state.mempool_fees.lock().await;

        mempool_fees.highest = rates.last().cloned();
        mempool_fees.last_in_block = last_in_block.cloned();
        mempool_fees.middle_in_block = middle_in_block.cloned();
        mempool_fees.transactions = NonZeroU32::new(block_template.len() as u32 + 1);

        drop(mempool_fees);

        sleep(tokio::time::Duration::from_secs(2)).await;

        log::trace!("mempool tx with fee: {}", rates.len());
    }
}
