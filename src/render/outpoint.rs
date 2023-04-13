use super::Html;
use crate::{globals::network, pages::tx::IO_PER_PAGE, NetworkExt};
use maud::{html, Render};

pub(crate) struct OutPoint(bitcoin::OutPoint);

impl Render for OutPoint {
    fn render(&self) -> maud::Markup {
        let txid_hex = format!("{:x}", self.0.txid);
        let network_url_path = network().as_url_path();
        let vout = self.0.vout;
        let page = vout as usize / IO_PER_PAGE;
        let page = if page == 0 {
            "".to_string()
        } else {
            format!("/{page}")
        };

        let link = format!("{network_url_path}t/{txid_hex}{page}#o{vout}");

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
