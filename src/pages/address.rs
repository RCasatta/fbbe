use std::{collections::HashMap, io::Cursor};

use base64::Engine;
use bitcoin::Address;
use maud::{html, Markup};
use qr_code::QrCode;

use crate::{
    error::Error,
    render::AmountRow,
    render::Html,
    req::ParsedRequest,
    route::convert_text_html_string,
    threads::index_addresses::{AddressOutputs, Balance},
};

use super::html_page;

pub fn page(
    address: &Address,
    parsed: &ParsedRequest,
    query: &Option<String>,
    address_outputs: Option<AddressOutputs>,
) -> Result<Markup, Error> {
    let (address_seen, balance) = match address_outputs {
        Some(AddressOutputs { seen, balance }) => (seen, Some(balance)),
        None => (vec![], None),
    };
    let address_type = address
        .address_type()
        .map(|t| t.to_string())
        .unwrap_or_else(|| "Unknown".to_owned());
    let mut params = match query {
        None => HashMap::new(),
        Some(q) => url::form_urlencoded::parse(q.as_bytes()).collect(),
    };
    params.retain(|_, v| !v.is_empty());
    let address_qr_uri = if params.is_empty() {
        format!("bitcoin:{:#}", address)
    } else {
        format!(
            "bitcoin:{:#}?{}",
            address,
            params
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<String>>()
                .join("&")
        )
    };

    let script_pubkey = address.script_pubkey();
    let txids_len = address_seen.len();

    // TODO the spent part
    //  eg 1 transaction output (1 spent)
    //  eg 1 transaction output
    //  eg 3 transaction outputs (1 spent)

    // TODO add truncated at the end

    // TODO paging to most recent 10 funding

    let content = html! {
        section {
            hgroup {
                h1 { "Address" }
                p  { (address.html()) }
            }

            @if !parsed.response_type.is_text() {
                p { a href=(&address_qr_uri) { img class="qr" alt="The QR-code of the bitcoin address" src=(create_bmp_base64_qr(&address_qr_uri)?); } }
            }

            table class="striped" {
                tbody {
                    tr {
                        th { "Type" }
                        td { (address_type) }
                    }
                    tr {
                        th { "Script" }
                        td { (script_pubkey.html()) }
                    }
                    @match balance {
                        Some(Balance::Known(sat)) => (AmountRow::new_with_sat(BALANCE, sat)),
                        Some(unknown) => tr {
                            th { (BALANCE) }
                            td { "unknown, " (unknown_reason(&unknown)) }
                        },
                        None => {}
                    }
                }
            }

            @if !address_seen.is_empty() {
                hgroup {
                    h2 { (txids_len) " transaction output" @if txids_len == 1 { "" } @else { "s" }  }
                    p { "Only confirmed outputs from the latest block paying this address are listed, older outputs, spent or unspent, are not shown" }
                }

                table class="striped" {
                    tbody {
                        @for (i, txid) in address_seen.iter().enumerate() {
                            @if i > 0 && parsed.response_type.is_text() {
                                tr { td { "\u{a0}" } } // without table borders entries would touch
                            }
                            tr {
                                td {
                                    (txid)
                                }
                            }
                        }
                    }
                    @if txids_len > 9 {
                        tfoot {
                            tr {
                                td { "possibly truncated"  }

                            }
                        }
                    }
                }
            }
        }
    };

    Ok(html_page("Address", content, parsed))
}

const BALANCE: &str = "Confirmed balance (BTC)";

fn unknown_reason(balance: &Balance) -> &'static str {
    match balance {
        Balance::Known(_) => "",
        Balance::IndexSyncing => "the address index is still syncing",
        Balance::MultipleFundingBlocks => "the address received funds in more than one block",
        Balance::TooManyOutputs => "too many outputs to check",
    }
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
    let bmp = qr.to_bmp().add_white_border(4)?.mul(3)?;

    let mut cursor = Cursor::new(vec![]);
    bmp.write(&mut cursor).unwrap();
    Ok(to_data_url(cursor.into_inner(), "image/bmp"))
}

/// No ASCII QR code in text: agents can't scan it and it is a lot of noise
pub fn text_page(page: &str, col: u16) -> Result<String, Error> {
    convert_text_html_string(page, col)
}
