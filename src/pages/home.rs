use super::html_page;
use crate::{
    network,
    render::Html,
    rpc::{chaininfo::ChainInfo, headers::HeightTime, mempool::MempoolInfo},
    state::MempoolFees,
};
use maud::{html, Markup};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use timeago::Formatter;

pub fn page(
    info: ChainInfo,
    height_time: HeightTime,
    mempool_info: MempoolInfo,
    mempool_fees: MempoolFees,
) -> Markup {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    let duration = Duration::from_secs(now.as_secs() - height_time.time as u64);
    let formatter = Formatter::new();
    let time_ago = formatter.convert(duration);
    let total_fees = format!("{:.8}", mempool_info.total_fee);
    let transaction_s = if mempool_info.size == 1 {
        "transaction"
    } else {
        "transactions"
    };
    let content = html! {
        section {

            hgroup {
                h1 { "Blockchain" }
                p { (format!("{:?}",network())) }
            }

            form {
                label for="s" { "Search for tx id, block height or hash" }
                input type="search" id="s" name="s" placeholder=(info.blocks) autofocus;
            }

            hgroup {
                h2 { "Latest block" }
                p { (info.blocks) }
            }

            table role="grid" {
                tbody {
                    tr {
                        th {
                            "Hash"
                        }
                        td class="right" {
                            (info.best_block_hash.html())
                        }
                    }
                    tr {
                        th {
                            "Elapsed"
                        }
                        td class="right" {
                            (time_ago)
                        }
                    }
                }
            }

            hgroup {
                h2 { "Mempool" }
                p { (mempool_info.size) " " (transaction_s) }
            }

            table role="grid" {
                tbody {
                    tr {
                        th { "Total fees (BTC)" }
                        td class="number" { (total_fees) }
                    }

                    (mempool_fees)

                }
            }

        }
    };

    html_page(&format!("{:?}", network()), content)
}
