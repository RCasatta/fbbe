use super::Html;
use crate::{
    render::{plural::Plural, AmountRow, SizeRow},
    rpc::mempool::MempoolInfo,
    state::BlockTemplate,
    threads::update_mempool_info::WeightFee,
};
use maud::{html, Render};

pub struct MempoolSection {
    pub info: MempoolInfo,
}

impl Render for MempoolSection {
    fn render(&self) -> maud::Markup {
        let transaction_s = Plural::new("transaction", self.info.size as usize);
        let mempoolminfee = self.info.mempoolminfee;

        html! {
            hgroup {
                h2 { "Mempool" }
                p { (self.info.size) " " (transaction_s) }
            }

            table class="striped" {
                tbody {
                    (AmountRow::new_with_btc("Total fees (BTC)", self.info.total_fee))

                    tr {
                        th { "Mempool min fee (BTC)" }
                        td class="number" { (WeightFee::from_btc_kvb(mempoolminfee)) }
                    }

                    // (SizeRow::new("Size (bytes)", self.info.bytes as usize))
                    (SizeRow::new("Memory usage", self.info.usage ))

                }
            }
        }
    }
}

impl Render for BlockTemplate {
    fn render(&self) -> maud::Markup {
        html! {

            @if let Some(transactions) = self.transactions.as_ref()  {
                hgroup {
                    h2 { "Block template" }
                    p { (transactions) " transactions" }
                }
                table class="striped" {
                    tbody {
                        @if let Some(highest) = self.highest.as_ref()  {
                            tr {
                                th  { "Highest" }
                                td class="number" { (highest.wf) }
                                td class="right" { (highest.txid.html()) }
                            }
                        }

                        @if let Some(middle_in_block) = self.middle_in_block.as_ref() {
                            tr {
                                th { "Middle" }
                                td class="number" { (middle_in_block.wf) }
                                td class="right" { (middle_in_block.txid.html()) }
                            }
                        }

                        @if let Some(last_in_block) = self.last_in_block.as_ref()  {
                            tr {
                                th { "Last " }
                                td class="number" { (last_in_block.wf) }
                                td class="right" { (last_in_block.txid.html()) }
                            }
                        }
                    }
                }
            }

        }
    }
}
