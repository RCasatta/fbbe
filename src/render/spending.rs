use maud::{html, Render};

use crate::{globals::network, threads::index_addresses::Spending, NetworkExt};

impl Render for Spending {
    fn render(&self) -> maud::Markup {
        let link = format!("{}t/{}#i{}", network().as_url_path(), self.txid, self.vin); //FIXME, broken for more than 9 (ling with page)

        html! {
            a href=(link) {
                code { span class="txid" { ( self.txid) } span class="vin" { ":" ( self.vin) } }
            }
        }
    }
}
