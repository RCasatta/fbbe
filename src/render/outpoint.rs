use std::fmt::Display;

use super::Html;
use crate::{globals::network, pages::tx::IO_PER_PAGE, NetworkExt};
use maud::{html, Render};

pub(crate) struct OutPoint(bitcoin::OutPoint);

struct Link<'a>(&'a bitcoin::OutPoint);
impl<'a> Display for Link<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}t/{}", network().as_url_path(), self.0.txid)?;
        let page = self.0.vout as usize / IO_PER_PAGE;
        if page > 0 {
            write!(f, "/{}", page)?;
        }
        write!(f, "#o{}", self.0.vout)
    }
}

impl Render for OutPoint {
    fn render(&self) -> maud::Markup {
        let link = Link(&self.0);

        html! {
            a href=(link) {
                code { span class="txid" { (self.0.txid) } span { ":" (self.0.vout) } }
            }
        }
    }
}

impl Html for bitcoin::OutPoint {
    fn html(&self) -> maud::Markup {
        OutPoint(*self).render()
    }
}
