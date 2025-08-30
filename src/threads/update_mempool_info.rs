use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::rpc;
use crate::state::{outpoints_and_sum, tx_output, OutPointsAndSum, SharedState, SpendPoint};
use bitcoin::{Txid, Weight};
use fxhash::FxHashSet;
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
        sleep(Duration::from_secs(2)).await;
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeightFeeCompact {
    pub weight: u32,
    pub fee: u32,
}

impl PartialOrd for WeightFeeCompact {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for WeightFeeCompact {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match self.rate().cmp(&other.rate()) {
            std::cmp::Ordering::Equal => self.weight.cmp(&other.weight),
            res => res,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TxidWeightFee {
    pub wf: WeightFee,
    pub txid: Txid,
}

#[derive(Debug, Clone, Eq)]
pub struct TxidWeightFeeCompact {
    pub wf: WeightFeeCompact,
    pub txid: Txid,
}

impl Ord for TxidWeightFeeCompact {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match self.wf.cmp(&other.wf) {
            std::cmp::Ordering::Equal => self.txid.cmp(&other.txid),
            res => res,
        }
    }
}

impl PartialOrd for TxidWeightFeeCompact {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for TxidWeightFeeCompact {
    fn eq(&self, other: &Self) -> bool {
        self.txid == other.txid && self.wf == other.wf
    }
}

impl Render for WeightFee {
    fn render(&self) -> maud::Markup {
        // em data-tooltip=(rate_sat_vb) style="font-style: normal" { (rate_btc_kvb)
        let btc_over_kvb = format!("{:.8}", self.rate_btc_over_kvb());
        let sat_over_vb = self.sat_over_vb_str();

        html! { span data-tooltip=(sat_over_vb) { (btc_over_kvb) } }
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
        (self.fee as f64 / 100_000.0) / (self.weight.to_wu() as f64 / 4.0)
    }

    /// for example `180.0` (sat/vB)
    fn rate_sat_over_vb(&self) -> f64 {
        (self.fee as f64) / (self.weight.to_wu() as f64 / 4.0)
    }

    pub fn sat_over_vb_str(&self) -> String {
        format!("{:.1} sat/vB", self.rate_sat_over_vb())
    }

    /// Built from a given rate, so that we can reuse the render functionality
    pub fn from_btc_kvb(f: f64) -> Self {
        Self {
            weight: Weight::from_wu(4_000),
            fee: (f * 100_000_000.0) as usize,
        }
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

    let mut rates: BTreeSet<TxidWeightFeeCompact> = BTreeSet::new();
    let mut rates_id: FxHashSet<Txid> = FxHashSet::default();
    let support_verbose = rpc::mempool::content(true).await.is_ok();
    log::info!("Node support compact mempool: {support_verbose}");

    loop {
        if let Ok(mempool) = rpc::mempool::content(support_verbose).await {
            rates.retain(|k| mempool.contains(&k.txid)); // keep only current mempool elements

            // keep only elements in the mempool
            shared_state
                .mempool_spending
                .lock()
                .await
                .retain(|_, v| mempool.contains(v.txid()));

            log::trace!("mempool content returns {} txids", mempool.len());

            let start = Instant::now();
            rates_id.clear();
            rates_id.extend(rates.iter().map(|e| e.txid));
            'outer: for txid in mempool.iter() {
                if rates_id.contains(txid) {
                    continue;
                }
                if let Ok((tx, _)) = shared_state.tx(*txid, false).await {
                    let OutPointsAndSum {
                        prevouts,
                        sum,
                        weight,
                    } = outpoints_and_sum(tx.as_ref()).expect("invalid tx bytes");

                    {
                        let mut mempool_spending = shared_state.mempool_spending.lock().await;
                        for (i, prevout) in prevouts.iter().enumerate() {
                            mempool_spending.insert(*prevout, SpendPoint::new(*txid, i as u32));
                        }
                    }

                    if prevouts.len() > 1 {
                        shared_state
                            .preload_prevouts_inner(*txid, prevouts.iter())
                            .await;
                    }

                    let mut sum_inputs = 0u64;
                    for prevout in prevouts.iter() {
                        if let Ok((prev_tx, _)) = shared_state.tx(prevout.txid, false).await {
                            let res = tx_output(prev_tx.as_ref(), prevout.vout, false)
                                .expect("invalid tx bytes");
                            sum_inputs += res.value.to_sat();
                        } else {
                            continue 'outer;
                        }
                    }
                    let fee = (sum_inputs - sum) as usize;
                    let wf = WeightFee { weight, fee };

                    if let Ok(wfc) = wf.try_into() {
                        rates.insert(TxidWeightFeeCompact {
                            wf: wfc,
                            txid: *txid,
                        });
                    }

                    if start.elapsed() > Duration::from_secs(60) {
                        log::info!(
                            "mempool info is taking more than a minute, breaking. Cache len: {} mempool: {}",
                            rates.len(),
                            mempool.len(),
                        );
                        break;
                    }
                }
            }
            let mut mempool_fees = shared_state.mempool_fees.lock().await;
            mempool_fees.mempool = mempool;
        } else {
            log::warn!("mempool content doesn't parse");
        }

        let mut sum = Weight::ZERO;
        let max = Weight::from_wu(4_000_000); // TODO use bitcoin::Weight::MAX_BLOCK once 0.31 released

        // TODO this doesn't take into account txs dependency
        let block_template_last = rates
            .iter()
            .rev()
            .enumerate()
            .take_while(|(_, e)| {
                sum += Weight::from_wu(e.wf.weight as u64);
                sum < max
            })
            .map(|(i, _)| i)
            .max();

        log::debug!("block template contains {:?}", block_template_last);

        let mut mempool_fees = shared_state.mempool_fees.lock().await;

        mempool_fees.highest = rates.last().map(Into::into);

        if let Some(n) = block_template_last {
            mempool_fees.last_in_block = rates.iter().nth_back(n).map(Into::into);
            mempool_fees.middle_in_block = rates.iter().nth_back(n / 2).map(Into::into);
            mempool_fees.transactions = Some(n + 1);
        }
        drop(mempool_fees);

        sleep(Duration::from_secs(10)).await;

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
