use std::time::Duration;

use super::html_page;
use crate::{
    network,
    render::{Html, MempoolSection, SizeRow},
    req::ParsedRequest,
    rpc::{chaininfo::ChainInfo, headers::HeightTime},
};
use maud::{html, Markup, PreEscaped};

const TWO_HOURS: Duration = Duration::from_secs(60 * 60 * 2);

pub fn page(
    info: ChainInfo,
    height_time: HeightTime,
    mempool_sec: MempoolSection,
    minutes_since_blocks: Option<String>,
    parsed: &ParsedRequest,
) -> Markup {
    let duration = height_time.since_now();
    let blockchain_size_row = SizeRow::new("Size on disk", info.size_on_disk);
    let content = html! {
        @if duration > TWO_HOURS {
            (PreEscaped("<!-- LAST BLOCK MORE THAN 2 HOURS AGO -->"))
        } @else {
            (PreEscaped("<!-- LAST BLOCK LESS THAN 2 HOURS AGO -->"))
        }
        section {

            hgroup {
                h1 { "Blockchain" }
                p { (format!("{:?}",network())) }
            }

            @if !parsed.response_type.is_text() {
                form {
                    label for="s" { "Search for tx id, block height or hash" }
                    input type="search" id="s" name="s" placeholder=(info.blocks) autofocus;
                }
            }

            table role="grid" {
                tbody {
                    tr {
                        th {
                            "Block " (info.blocks)
                        }
                        td class="right" {
                            (info.best_block_hash.html())
                        }
                    }

                    @if let Some(minutes_since_block) = minutes_since_blocks.as_ref() {
                        tr {
                            th {
                                "Minutes since last 6 blocks"
                            }
                            td class="right" {
                                (minutes_since_block)
                            }
                        }
                    }

                    (blockchain_size_row)

                }
            }


            (mempool_sec)

        }
    };

    html_page(&format!("{:?}", network()), content, parsed)
}
