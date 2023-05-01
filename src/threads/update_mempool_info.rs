use std::collections::HashMap;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::rpc;
use crate::state::SharedState;
use bitcoin::{Txid, Weight};
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
    pub weight: Weight,

    /// The absolute fee in satoshi
    pub fee: usize,
}

/// Used to save memory in a vec containing a lot of this elements, moreover:
/// - weight should never exceed u32::MAX, unless you are computing the weight of a blockchain
/// - fee has exceeded u32::MAX satoshi at least once cc455ae816e6cdafdb58d54e35d4f46d860047458eacf1c7405dc634631c570d
///   but it's a very rare case should almost never happen
#[derive(Debug, Clone)]
pub struct WeightFeeCompact {
    pub weight: u32,

    pub fee: u32,
}

#[derive(Debug, Clone)]
pub struct TxidWeightFee {
    pub wf: WeightFee,
    pub txid: Txid,
}

#[derive(Debug, Clone)]
pub struct TxidWeightFeeCompact {
    pub wf: WeightFeeCompact,
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

impl From<WeightFeeCompact> for WeightFee {
    fn from(value: WeightFeeCompact) -> Self {
        From::from(&value)
    }
}

impl From<&WeightFeeCompact> for WeightFee {
    fn from(value: &WeightFeeCompact) -> Self {
        Self {
            weight: Weight::from_wu(value.weight as u64),
            fee: value.fee as usize,
        }
    }
}

impl TryFrom<WeightFee> for WeightFeeCompact {
    type Error = std::num::TryFromIntError;

    fn try_from(value: WeightFee) -> Result<Self, Self::Error> {
        Ok(Self {
            weight: u32::try_from(value.weight.to_wu())?,
            fee: u32::try_from(value.fee)?,
        })
    }
}

impl From<&TxidWeightFeeCompact> for TxidWeightFee {
    fn from(value: &TxidWeightFeeCompact) -> Self {
        Self {
            wf: (&value.wf).into(),
            txid: value.txid,
        }
    }
}

impl WeightFee {
    /// for example `0.00179955` (BTC/KvB)
    fn rate_btc_over_kvb(&self) -> f64 {
        (self.fee as f64 / 100_000_000.0) / (self.weight.to_wu() as f64 / 4_000.0)
    }

    /// for example `180.0` (sat/vB)
    fn rate_sat_over_vb(&self) -> f64 {
        (self.fee as f64) / (self.weight.to_wu() as f64 / 4.0)
    }

    pub fn sat_over_vb_str(&self) -> String {
        format!("{:.1} sat/vB", self.rate_sat_over_vb())
    }
}

impl WeightFeeCompact {
    /// Fast computing (integer math) rate, used for sorting
    fn rate(&self) -> u64 {
        ((self.fee as u64) << 32) / self.weight as u64
    }
}

async fn update_mempool_details(shared_state: Arc<SharedState>) {
    log::info!("Starting update_mempool_details");

    let mut cache: HashMap<Txid, WeightFeeCompact> = HashMap::new();
    let mut rates: Vec<TxidWeightFeeCompact> = vec![];

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
                    let weight = tx.weight();
                    let wf = WeightFee { weight, fee };

                    if let Ok(wf) = wf.try_into() {
                        cache.insert(txid, wf);
                    }

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
                .map(|(txid, wf)| TxidWeightFeeCompact { wf, txid }),
        );
        rates.sort_by_cached_key(|a| a.wf.rate());

        let mut sum = Weight::ZERO;
        let max = Weight::from_wu(4_000_000);

        // TODO this doesn't take into account txs dependency
        let block_template: Vec<_> = rates
            .iter()
            .rev()
            .take_while(|i| {
                sum += Weight::from_wu(i.wf.weight as u64);
                sum < max
            })
            .collect();
        log::debug!("block template contains {}", block_template.len());

        let last_in_block = block_template.last().cloned();
        let middle_in_block = block_template.get(block_template.len() / 2).cloned();

        let mut mempool_fees = shared_state.mempool_fees.lock().await;

        mempool_fees.highest = rates.last().map(Into::into);
        mempool_fees.last_in_block = last_in_block.map(Into::into);
        mempool_fees.middle_in_block = middle_in_block.map(Into::into);
        mempool_fees.transactions = NonZeroU32::new(block_template.len() as u32 + 1);

        drop(mempool_fees);

        sleep(tokio::time::Duration::from_secs(2)).await;

        log::trace!("mempool tx with fee: {}", rates.len());
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn size_weight_fee() {
        assert_eq!(size_of::<WeightFee>(), 16);
        assert_eq!(size_of::<WeightFeeCompact>(), 8);
        assert_eq!(size_of::<TxidWeightFee>(), 48);
        assert_eq!(size_of::<TxidWeightFeeCompact>(), 40);
    }
}
