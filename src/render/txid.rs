use super::Html;
use crate::{globals::network, NetworkExt};
use bitcoin::hashes::hex::ToHex;
use maud::{html, Render};

pub(crate) struct Txid(bitcoin::Txid);

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
