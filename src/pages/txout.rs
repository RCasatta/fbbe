use bitcoin::OutPoint;
use maud::{html, Markup};

use crate::{render::Html, req::ParsedRequest, rpc::txout::TxOutJson};

use super::html_page;

pub fn page(tx: &TxOutJson, outpoint: OutPoint, parsed: &ParsedRequest) -> Markup {
    let is_spent = if tx.utxos.is_empty() {
        "SPENT"
    } else {
        "UNSPENT"
    };

    let content = html! {
        section {
            hgroup {
                h1 { "Transaction output " }
                p {(outpoint.html()) }
            }

            h2 { (is_spent) }

        }
    };

    html_page("Txout", content, parsed)
}
