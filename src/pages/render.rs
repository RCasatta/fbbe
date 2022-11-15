use crate::{network, state::MempoolFees, NetworkExt};
use bitcoin::hashes::hex::ToHex;
use maud::{html, Render};

pub trait Html {
    fn html(&self) -> maud::Markup;
}

struct Txid(bitcoin::Txid);

impl Render for Txid {
    fn render(&self) -> maud::Markup {
        let hex = self.0.to_hex();
        let network_url_path = network().as_url_path();
        let link = format!("{network_url_path}t/{hex}");

        html! {
            a href=(link) { code { small { u { (hex) } } } }
        }
    }
}

impl Html for bitcoin::Txid {
    fn html(&self) -> maud::Markup {
        Txid(*self).render()
    }
}

impl From<bitcoin::Txid> for Txid {
    fn from(t: bitcoin::Txid) -> Self {
        Txid(t)
    }
}

pub struct BlockHash(pub bitcoin::BlockHash);

impl Render for BlockHash {
    fn render(&self) -> maud::Markup {
        let hex = self.0.to_hex();
        let network_url_path = network().as_url_path();
        let link = format!("{network_url_path}b/{hex}");

        html! {
            a href=(link) { code { small { (hex) } } }
        }
    }
}

impl Html for bitcoin::BlockHash {
    fn html(&self) -> maud::Markup {
        BlockHash(*self).render()
    }
}

impl From<bitcoin::BlockHash> for BlockHash {
    fn from(t: bitcoin::BlockHash) -> Self {
        BlockHash(t)
    }
}

struct OutPoint(bitcoin::OutPoint);

impl Render for OutPoint {
    fn render(&self) -> maud::Markup {
        let txid_hex = self.0.txid.to_hex();
        let network_url_path = network().as_url_path();
        let vout = self.0.vout;

        let link = format!("{network_url_path}t/{txid_hex}#o{vout}");

        html! {
            a href=(link) {
                code { u { (txid_hex) } ":" b { (vout) } }
            }
        }
    }
}

impl Html for bitcoin::OutPoint {
    fn html(&self) -> maud::Markup {
        OutPoint(*self).render()
    }
}

impl Render for MempoolFees {
    fn render(&self) -> maud::Markup {
        html! {

            @if let Some(highest) = self.highest  {
                tr {
                    th { em data-tooltip="Transaction with the highest fee in the mempool" { "Highest" } }
                    td class="right" { (highest.html()) }
                }
            }

            @if let Some(middle_in_block) = self.middle_in_block  {
                tr {
                    th { em data-tooltip="A transaction in the middle of a block template created with current mempool" { "Middle in block" } }
                    td class="right" { (middle_in_block.html()) }
                }
            }

            @if let Some(last_in_block) = self.last_in_block  {
                tr {
                    th { em data-tooltip="The last transaction (lowest fee) of a block template created with current mempool" { "Last in block" } }
                    td class="right" { (last_in_block.html()) }
                }
            }


        }
    }
}
