use super::html_page;
use crate::{
    network,
    render::{Html, MempoolSection},
    route::ResponseType,
    rpc::{chaininfo::ChainInfo, headers::HeightTime},
};
use maud::{html, Markup};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use timeago::Formatter;

pub fn page(
    info: ChainInfo,
    height_time: HeightTime,
    mempool_sec: MempoolSection,
    response_type: ResponseType,
) -> Markup {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    let duration = Duration::from_secs(now.as_secs() - height_time.time as u64);
    let formatter = Formatter::new();
    let time_ago = formatter.convert(duration);

    let content = html! {
        section {

            hgroup {
                h1 { "Blockchain" }
                p { (format!("{:?}",network())) }
            }

            @if !response_type.is_text() {
                form {
                    label for="s" { "Search for tx id, block height or hash" }
                    input type="search" id="s" name="s" placeholder=(info.blocks) autofocus;
                }
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

            (mempool_sec)

        }
    };

    html_page(&format!("{:?}", network()), content, response_type)
}
