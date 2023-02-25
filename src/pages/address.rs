use std::io::Cursor;

use base64::Engine;
use bitcoin::Address;
use maud::{html, Markup};
use qr_code::QrCode;

use crate::{error::Error, render::Html, req::ParsedRequest};

use super::html_page;

pub fn page(address: &Address, parsed: &ParsedRequest) -> Result<Markup, Error> {
    let mempool = format!("https://mempool.space/address/{address}");
    let blockstream = format!("https://blockstream.info/address/{address}");
    let address_type = address
        .address_type()
        .map(|t| t.to_string())
        .unwrap_or("Unknown".to_string());
    let image_src = create_bmp_base64_qr(&address.to_qr_uri())?;

    let content = html! {
        section {
            hgroup {
                h1 { "Address" }
                p  { (address.html()) }
            }

            p { "Type: " b { (address_type) } }

            p { img class="qr" src=(image_src); }

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
