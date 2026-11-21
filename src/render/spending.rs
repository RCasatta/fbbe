use maud::{html, Render};

use crate::{
    render::{tx_io_link, Io},
    threads::index_addresses::Spending,
};

impl Render for Spending {
    fn render(&self) -> maud::Markup {
        let link = tx_io_link(&self.txid, Io::Input, self.vin);

        html! {
            a href=(link) {
                code { span class="txid" { ( self.txid) } span class="vin" { ":" ( self.vin) } }
            }
        }
    }
}
