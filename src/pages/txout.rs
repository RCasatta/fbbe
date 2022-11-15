use bitcoin::OutPoint;
use bitcoin_hashes::hex::ToHex;
use maud::{html, Markup};

use crate::{network, rpc::txout::TxOutJson, NetworkExt};

use super::html_page;

pub fn page(tx: &TxOutJson, outpoint: OutPoint) -> Markup {
    let txid = outpoint.txid.to_hex();
    let outpoint = html! { code { u { (&txid) } ":" (outpoint.vout) } };
    let link = format!("{}t/{}", network().as_url_path(), &txid);
    let is_spent = if tx.utxos.is_empty() {
        "SPENT"
    } else {
        "UNSPENT"
    };

    let content = html! {
        section {
            hgroup {
                h1 { "Transaction output " }
                p { a href=(link) { (outpoint) } }
            }

            h2 { (is_spent) }

        }
    };

    html_page("Txout", content)
}
