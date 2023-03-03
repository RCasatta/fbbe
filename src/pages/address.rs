use std::io::Cursor;

use base64::Engine;
use bitcoin::Address;
use maud::{html, Markup};
use qr_code::QrCode;

use crate::{
    error::Error, globals::network, render::Html, req::ParsedRequest,
    route::convert_text_html_string,
};

use super::html_page;

pub fn page(address: &Address, parsed: &ParsedRequest) -> Result<Markup, Error> {
    use bitcoin::Network::*;
    let network = network();
    let network_path = match network {
        Bitcoin => "",
        Testnet => "testnet/",
        Signet => "signet/",
        Regtest => "regtest/",
    };
    let mempool = match network {
        Bitcoin | Testnet | Signet => Some(format!(
            "https://mempool.space/{network_path}address/{address}"
        )),
        _ => None,
    };
    let blockstream = match network {
        Bitcoin | Testnet => Some(format!(
            "https://blockstream.info/{network_path}address/{address}"
        )),
        _ => None,
    };
    let address_type = address
        .address_type()
        .map(|t| t.to_string())
        .unwrap_or_else(|| "Unknown".to_owned());
    let address_qr_uri = address.to_qr_uri();
    let script_pubkey = address.script_pubkey();

    let content = html! {
        section {
            hgroup {
                h1 { "Address" }
                p  { (address.html()) }
            }

            @if !parsed.response_type.is_text() {
                p { a href=(&address_qr_uri) { img class="qr" src=(create_bmp_base64_qr(&address_qr_uri)?); } }
            }

            table role="grid" {
                tbody {
                    tr {
                        th { "Type" }
                        td { (address_type) }
                    }
                    tr {
                        th { "Script" }
                        td { (script_pubkey.html()) }
                    }
                }
            }

            @if !parsed.response_type.is_text() && (mempool.is_some() || blockstream.is_some()) {
                p {
                    "This explorer doesn't index addresses. Check the following explorers:"

                    ul {
                        @if let Some(mempool) = mempool {
                            li { a href=(mempool) { "mempool.space" } }
                        }
                        @if let Some(blockstream) = blockstream {
                            li { a href=(blockstream) { "blockstream.info" } }
                        }

                    }
                }
            }

        }
    };

    Ok(html_page("Address", content, parsed))
}

/// Converts `input` in base64 and returns a data url
pub fn to_data_url<T: AsRef<[u8]>>(input: T, content_type: &str) -> String {
    let base64 = base64::engine::general_purpose::STANDARD_NO_PAD.encode(input.as_ref());
    format!("data:{};base64,{}", content_type, base64)
}

/// Creates QR containing `message` and encode it in data url
fn create_bmp_base64_qr(message: &str) -> Result<String, Error> {
    let qr = QrCode::new(message.as_bytes())?;

    // The `.mul(3)` with pixelated rescale shouldn't be needed, however, some printers doesn't
    // recognize it resulting in a blurry image, starting with a bigger image mostly prevents the
    // issue at the cost of a bigger image size.
    let bmp = qr.to_bmp().add_white_border(2)?.mul(3)?;

    let mut cursor = Cursor::new(vec![]);
    bmp.write(&mut cursor).unwrap();
    Ok(to_data_url(cursor.into_inner(), "image/bmp"))
}

pub fn text_page(address: &Address, page: &str, col: u16) -> Result<String, Error> {
    let mut s = convert_text_html_string(page, col);
    s.push('\n');
    s.push_str(&create_string_qr(&address.to_qr_uri())?);
    Ok(s)
}
/// Creates QR containing `message` and encode it in data url
pub(crate) fn create_string_qr(message: &str) -> Result<String, Error> {
    let qr = QrCode::new(message.as_bytes())?;

    Ok(qr.to_string(true, 2))
}
