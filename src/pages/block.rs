use crate::{
    error::Error,
    network,
    pages::{html_page, size_rows},
    render::{self, AmountRow, Html, Plural},
    req::ParsedRequest,
    rpc::block::BlockNoTxDetails,
    NetworkExt,
};
use maud::{html, Markup};

const PER_PAGE: usize = 10;

/// Block reward without fees, in satoshi
pub fn subsidy(height: u32, network: bitcoin::Network) -> u64 {
    let halving_interval = match network {
        bitcoin::Network::Regtest => 150,
        _ => 210_000,
    };
    let halvings = height / halving_interval;
    if halvings >= 64 {
        0
    } else {
        (50 * 100_000_000u64) >> halvings
    }
}

/// `coinbase_value` is the sum of the coinbase outputs, in satoshi
pub fn page(
    block: &BlockNoTxDetails,
    page: usize,
    coinbase_value: u64,
    parsed: &ParsedRequest,
) -> Result<Markup, Error> {
    let from_tx = page * PER_PAGE;
    if from_tx >= block.tx.len() {
        return Err(Error::InvalidPageNumber);
    }
    let to_tx = block.tx.len().min(from_tx + PER_PAGE);
    let network_url_path = network().as_url_path();
    let txids = block.tx.iter().skip(from_tx).take(PER_PAGE).enumerate();
    let translate = |i: usize| i + from_tx;
    let transaction_plural = Plural::new("transaction", block.tx.len());

    let prev_txs = (page > 0).then(|| format!("{}b/{}/{}", network_url_path, block.hash, page - 1));
    let next_txs = (to_tx != block.tx.len())
        .then(|| format!("{}b/{}/{}", network_url_path, block.hash, page + 1));
    let separator_txs = (prev_txs.is_some() && next_txs.is_some()).then_some(" | ");

    let current_block = if page == 0 {
        html! { (block.height) }
    } else {
        let block_link = format!("{}b/{}", network().as_url_path(), block.hash);
        html! {a href=(block_link) {(block.height)}}
    };

    let content = html! {
        section {
            hgroup {
                h1 { "Block " (current_block) }
                p { (block.previous_block_hash_link()) (render::BlockHash::from((block.hash, false))) (block.next_block_hash_link()) }
            }

            table class="striped" {
                tbody {
                    tr {
                        th { "Timestamp" }
                        td class="right" { (block.date_time_utc()) }
                    }
                    (size_rows(block.size, block.weight))
                    (AmountRow::new_with_sat("Coinbase value (BTC)", coinbase_value))
                    // miners may claim less than allowed, it happened rarely in early blocks
                    (AmountRow::new_with_sat("Fees (BTC)", coinbase_value.saturating_sub(subsidy(block.height, network()))))
                }
            }

            hgroup {
                h2 { (block.tx.len()) " " (transaction_plural) }
                p {
                    @if let Some(prev) = prev_txs {
                        a href=(prev) { "Prev" }
                    }
                    @if let Some(separator) = separator_txs {
                        (separator)
                    }
                    @if let Some(next) = next_txs {
                        a href=(next) { "Next" }
                    }
                }
            }

            table class="striped" {
                tbody {
                    @for (i, txid) in txids {
                        tr {
                            th class="row-index" {
                                (translate(i))
                            }
                            td {
                                (txid.html())
                            }
                        }
                    }
                }
            }

            h2 { "Details" }

            table class="striped" {
                tbody {

                    tr {
                        th { "Version" }
                        td class="right" { "0x" (block.version_hex) }
                    }
                    tr {
                        th { "Merkle root" }
                        td class="right" { code { (block.merkleroot) } }
                    }
                    tr {
                        th { "Bits" }
                        td class="right" { "0x" (block.bits) }
                    }
                    tr {
                        th { "Difficulty" }
                        td class="right" { (block.difficulty) }
                    }
                    tr {
                        th { "Nonce" }
                        td class="right" { (block.nonce) }
                    }
                }
            }
        }
    };

    Ok(html_page("Block", content, parsed))
}

#[cfg(test)]
mod tests {
    use super::subsidy;
    use bitcoin::Network;

    #[test]
    fn block_subsidy() {
        assert_eq!(subsidy(0, Network::Bitcoin), 5_000_000_000);
        assert_eq!(subsidy(209_999, Network::Bitcoin), 5_000_000_000);
        assert_eq!(subsidy(210_000, Network::Bitcoin), 2_500_000_000);
        assert_eq!(subsidy(840_000, Network::Bitcoin), 312_500_000);
        assert_eq!(subsidy(150, Network::Regtest), 2_500_000_000);
        assert_eq!(subsidy(64 * 210_000, Network::Bitcoin), 0);
    }
}
