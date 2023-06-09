use std::fmt::Display;

use super::Html;
use crate::{globals::network, NetworkExt};
use maud::{html, Render};

pub(crate) struct BlockHash(pub bitcoin::BlockHash);

struct Link(bitcoin::BlockHash);
impl Display for Link {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}b/{:x}", network().as_url_path(), self.0)
    }
}

impl Render for BlockHash {
    fn render(&self) -> maud::Markup {
        let link = Link(self.0);

        html! {
            a href=(link) { code { (self.0) } }
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
