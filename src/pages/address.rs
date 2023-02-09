use bitcoin::Address;
use maud::{html, Markup};

use crate::{error::Error, render::Html};

use super::html_page;

pub fn page(address: &Address) -> Result<Markup, Error> {
    let mempool = format!("https://mempool.space/address/{address}");
    let blockstream = format!("https://blockstream.info/address/{address}");

    let content = html! {
        section {
            hgroup {
                h1 { "Address" }
                p  { (address.html()) }
            }

            p {
                "This explorer doesn't index addresses. Check the following explorers:"

                ul {
                    li { a href=(mempool) { "mempool.space" } }
                    li { a href=(blockstream) { "blockstream.info" } }

                }
            }

        }
    };

    Ok(html_page("Address", content))
}
