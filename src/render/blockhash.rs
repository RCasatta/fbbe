use super::Html;
use crate::{globals::network, NetworkExt};
use maud::{html, Render};

pub(crate) struct BlockHash(pub bitcoin::BlockHash);

impl Render for BlockHash {
    fn render(&self) -> maud::Markup {
        let hex = format!("{:x}", self.0);
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
