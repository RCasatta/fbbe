use super::Html;
use crate::{
    render::{plural::Plural, AmountRow, SizeRow},
    rpc::mempool::MempoolInfo,
    state::BlockTemplate,
};
use maud::{html, Render};

pub struct MempoolSection {
    pub info: MempoolInfo,
}

impl Render for MempoolSection {
    fn render(&self) -> maud::Markup {
        let transaction_s = Plural::new("transaction", self.info.size as usize);
        let mempoolminfee = if self.info.mempoolminfee > 0.00000999 {
            Some(self.info.mempoolminfee)
        } else {
            None
        };

        html! {
            hgroup {
                h2 { "Mempool" }
                p { (self.info.size) " " (transaction_s) }
            }

            table role="grid" {
                tbody {
                    (AmountRow::new_with_btc("Total fees (BTC)", self.info.total_fee))

                    @if let Some(mempoolminfee) = mempoolminfee {
                        (AmountRow::new_with_btc("Mempool min fee (BTC)", mempoolminfee))
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
                table role="grid" {
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
