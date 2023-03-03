use super::Html;
use crate::{
    render::{AmountRow, SizeRow},
    rpc::mempool::MempoolInfo,
    state::MempoolFees,
};
use maud::{html, Render};

pub struct MempoolSection {
    pub info: MempoolInfo,
    pub fees: MempoolFees,
}

impl Render for MempoolSection {
    fn render(&self) -> maud::Markup {
        let transaction_s = if self.info.size == 1 {
            "transaction"
        } else {
            "transactions"
        };
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
                    (SizeRow::new("Memory usage (bytes)", self.info.usage as usize))


                    (self.fees)

                }
            }
        }
    }
}

impl Render for MempoolFees {
    fn render(&self) -> maud::Markup {
        html! {

            @if let Some(highest) = self.highest.as_ref()  {
                tr {
                    th { em data-tooltip="Transaction with the highest fee in the mempool" { "Highest" } }
                    td class="right" { (highest.txid.html()) }
                }
            }

            @if let Some(middle_in_block) = self.middle_in_block.as_ref() {
                tr {
                    th { em data-tooltip="A transaction in the middle of a block template created with current mempool" { "Middle in block" } }
                    td class="right" { (middle_in_block.txid.html()) }
                }
            }

            @if let Some(last_in_block) = self.last_in_block.as_ref()  {
                tr {
                    th { em data-tooltip="The last transaction (lowest fee) of a block template created with current mempool" { "Last in block" } }
                    td class="right" { (last_in_block.txid.html()) }
                }
            }


        }
    }
}
