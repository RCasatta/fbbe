use bitcoin::Address;
use maud::{html, Markup};

use crate::{error::Error, render::Html, req::ParsedRequest};

use super::html_page;

pub fn page(address: &Address, parsed: &ParsedRequest) -> Result<Markup, Error> {
    let mempool = format!("https://mempool.space/address/{address}");
    let blockstream = format!("https://blockstream.info/address/{address}");
    let address_type = address
        .address_type()
        .map(|t| t.to_string())
        .unwrap_or("Unknown".to_string());

    let content = html! {
        section {
            hgroup {
                h1 { "Address" }
                p  { (address.html()) }
            }

            p { "Type: " b { (address_type) } }

            p {
                "This explorer doesn't index addresses. Check the following explorers:"

                ul {
                    li { a href=(mempool) { "mempool.space" } }
                    li { a href=(blockstream) { "blockstream.info" } }

                }
            }

        }
    };

    Ok(html_page("Address", content, parsed))
}
