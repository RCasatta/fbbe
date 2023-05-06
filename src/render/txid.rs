use std::fmt::Display;

use super::Html;
use crate::{globals::network, NetworkExt, NetworkPath};
use maud::{html, Render};

pub(crate) struct Txid(bitcoin::Txid);

struct Link(NetworkPath, bitcoin::Txid);
impl Display for Link {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}t/{:x}", self.0, self.1)
    }
}

impl Render for Txid {
    fn render(&self) -> maud::Markup {
        let network_url_path = network().as_url_path();
        let link = Link(network_url_path, self.0);

        html! {
            a href=(link) { code { small { u { (self.0) } } } }
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
