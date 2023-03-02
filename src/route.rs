use crate::{
    error::Error,
    network, pages,
    render::MempoolSection,
    req::{self, Resource},
    rpc, NetworkExt, SharedState,
};
use bitcoin::{consensus::serialize, Network, OutPoint, TxOut, Txid};
use bitcoin_hashes::Hash;
use html2text::render::text_renderer::RichDecorator;
use hyper::{
    body::Bytes,
    header::{CACHE_CONTROL, CONTENT_TYPE, IF_MODIFIED_SINCE, LAST_MODIFIED, LOCATION},
    Body, Request, Response, StatusCode,
};
use mime::{APPLICATION_OCTET_STREAM, TEXT_HTML_UTF_8, TEXT_PLAIN_UTF_8};
use std::{convert::Infallible, sync::Arc, time::Instant};

const CSS_LAST_MODIFIED: &str = "2022-10-03 07:53:03 UTC";
const CONTACT_PAGE_LAST_MODIFIED: &str = "2022-12-16 07:53:03 UTC";
const ROBOTS_LAST_MODIFIED: &str = "2023-01-17 07:53:03 UTC";

#[derive(Debug, Clone, Copy)]
pub enum ResponseType {
    Text(usize),
    Html,
    Bytes,
}

impl ResponseType {
    pub fn is_text(&self) -> bool {
        match self {
            ResponseType::Text(_) => true,
            _ => false,
        }
    }
}

pub async fn route(req: Request<Body>, state: Arc<SharedState>) -> Result<Response<Body>, Error> {
    let now = Instant::now();
    // let _count = state.requests.fetch_add(1, Ordering::Relaxed);
    let parsed_req = req::parse(&req).await?;

    // DETERMINE IF NOT MODIFIED
    if let Some(if_modified_since) = req.headers().get(IF_MODIFIED_SINCE) {
        log::trace!("{:?} if modified since {:?}", req.uri(), if_modified_since);
        let modified = match &parsed_req.resource {
            // Resource::Tx(txid) => state.txs.lock().await.get(txid).map,
            Resource::Block(block_hash, _) => state
                .hash_to_height_time
                .lock()
                .await
                .get(block_hash)
                .map(|e| e.date_time_utc()),
            Resource::Tx(txid, _) => {
                if let Some(block_hash) = state.tx_in_block.lock().await.get(txid) {
                    state
                        .hash_to_height_time
                        .lock()
                        .await
                        .get(block_hash)
                        .map(|e| e.date_time_utc())
                } else {
                    None
                }
            }
            Resource::Css => Some(CSS_LAST_MODIFIED.to_string()),
            Resource::Contact => Some(CONTACT_PAGE_LAST_MODIFIED.to_string()),

            _ => None,
        };
        if let Some(modified) = modified {
            if *if_modified_since == modified {
                log::debug!("{:?} Not modified", req.uri());

                return Ok(Response::builder()
                    .status(StatusCode::NOT_MODIFIED)
                    .body(Body::empty())?);
            }
        }
    }

    let resp = match parsed_req.resource {
        Resource::Home => {
            let chain_info = state.chain_info.lock().await.clone();
            let info = state.mempool_info.lock().await.clone();
            let fees = state.mempool_fees.lock().await.clone();
            let mempool_section = MempoolSection { info, fees };

            let height_time = state.height_time(chain_info.best_block_hash).await?;
            let page = pages::home::page(chain_info, height_time, mempool_section, &parsed_req)
                .into_string();

            let builder = Response::builder().header(CACHE_CONTROL, "public, max-age=5");
            match parsed_req.response_type {
                ResponseType::Text(col) => builder
                    .header(CONTENT_TYPE, TEXT_PLAIN_UTF_8.as_ref())
                    .body(convert_text_html(&page, col))?,
                ResponseType::Html => builder
                    .header(CONTENT_TYPE, TEXT_HTML_UTF_8.as_ref())
                    .body(page.into())?,
                ResponseType::Bytes => {
                    return Err(Error::ContentTypeUnsupported(
                        parsed_req.response_type,
                        req.uri().to_string(),
                    ))
                }
            }
        }

        Resource::Block(block_hash, page) => {
            let block = rpc::block::call_json(block_hash).await?;
            let page = pages::block::page(&block, page, &parsed_req)?.into_string();
            let current_tip = state.chain_info.lock().await.clone();
            let block_confirmations = current_tip.blocks - block.height;
            let cache_seconds = cache_time_from_confirmations(Some(block_confirmations));
            let cache_control = format!("public, max-age={cache_seconds}");

            let builder = Response::builder()
                .header(CACHE_CONTROL, cache_control) // cache examples https://developers.cloudflare.com/cache/about/cache-control/#examples
                .header(LAST_MODIFIED, block.date_time_utc());

            match parsed_req.response_type {
                ResponseType::Text(col) => builder
                    .header(CONTENT_TYPE, TEXT_PLAIN_UTF_8.as_ref())
                    .body(convert_text_html(&page, col))?,
                ResponseType::Html => builder
                    .header(CONTENT_TYPE, TEXT_HTML_UTF_8.as_ref())
                    .body(page.into())?,
                ResponseType::Bytes => {
                    return Err(Error::ContentTypeUnsupported(
                        parsed_req.response_type,
                        req.uri().to_string(),
                    ))
                }
            }
        }

        Resource::Tx(txid, pagination) => {
            if pagination > 0 {
                if let ResponseType::Bytes = parsed_req.response_type {
                    return Err(Error::BadRequest);
                }
            }
            let (tx, block_hash) = state.tx(txid, true).await?;
            let ts = match block_hash.as_ref() {
                Some(block_hash) => Some((*block_hash, state.height_time(*block_hash).await?)),
                None => None,
            };
            let prevouts = fetch_prevouts(&tx, &state, false).await?;
            let current_tip = state.chain_info.lock().await.clone();
            let mempool_fees = state.mempool_fees.lock().await.clone();
            let page = pages::tx::page(
                &tx,
                ts,
                &prevouts,
                pagination,
                mempool_fees,
                &parsed_req,
                false,
            )?
            .into_string();
            let cache_seconds =
                cache_time_from_confirmations(ts.map(|t| current_tip.blocks - t.1.height));

            let cache_control = format!("public, max-age={cache_seconds}");
            let mut builder = Response::builder().header(CACHE_CONTROL, cache_control);
            if let Some(ts) = ts {
                builder = builder.header(LAST_MODIFIED, ts.1.date_time_utc());
            }

            match parsed_req.response_type {
                ResponseType::Text(col) => builder
                    .header(CONTENT_TYPE, TEXT_PLAIN_UTF_8.as_ref())
                    .body(convert_text_html(&page, col))?,
                ResponseType::Html => builder
                    .header(CONTENT_TYPE, TEXT_HTML_UTF_8.as_ref())
                    .body(page.into())?,
                ResponseType::Bytes => builder
                    .header(CONTENT_TYPE, APPLICATION_OCTET_STREAM.as_ref())
                    .body(Bytes::from(serialize(&tx)).into())?,
            }
        }

        Resource::TxOut(txid, vout) => match rpc::txout::call(txid, vout).await {
            Ok(tx) => {
                let outpoint = OutPoint::new(txid, vout);
                let page = pages::txout::page(&tx, outpoint, &parsed_req).into_string();
                let cache_seconds = if tx.utxos.is_empty() {
                    60 * 60 * 24 * 30 // one month
                } else {
                    5
                };
                Response::builder()
                    .header(CONTENT_TYPE, "text/html; charset=utf-8")
                    .header(CACHE_CONTROL, format!("public, max-age={cache_seconds}"))
                    .body(page.into())?
            }
            Err(e) => return Err(e),
        },

        Resource::SearchHeight(height) => {
            let hash = state.hash(height).await?;
            let network = network().as_url_path();

            Response::builder()
                .header(LOCATION, format!("{network}b/{hash}"))
                .status(StatusCode::TEMPORARY_REDIRECT) // PERMANENT_REDIRECT cause issues in lynx
                .body(Body::empty())?
        }

        Resource::SearchBlock(hash) => {
            let network = network().as_url_path();
            Response::builder()
                .header(LOCATION, format!("{network}b/{hash}"))
                .status(StatusCode::TEMPORARY_REDIRECT) // PERMANENT_REDIRECT cause issues in lynx
                .body(Body::empty())?
        }

        Resource::SearchTx(txid) => {
            let network = network().as_url_path();
            Response::builder()
                .header(LOCATION, format!("{network}t/{txid}"))
                .status(StatusCode::TEMPORARY_REDIRECT)
                .body(Body::empty())?
        }

        Resource::SearchAddress(address) => {
            let network = network().as_url_path();
            Response::builder()
                .header(LOCATION, format!("{network}a/{address}"))
                .status(StatusCode::TEMPORARY_REDIRECT)
                .body(Body::empty())?
        }

        Resource::Head => Response::new(Body::empty()),

        Resource::Css => Response::builder()
            .header(LAST_MODIFIED, CSS_LAST_MODIFIED)
            .header(CACHE_CONTROL, "public, max-age=31536000")
            .header(CONTENT_TYPE, "text/css; charset=utf-8")
            .body(Body::from(include_str!("css/pico.min.css")))?,

        Resource::Contact => Response::builder()
            .header(LAST_MODIFIED, CONTACT_PAGE_LAST_MODIFIED)
            .header(CACHE_CONTROL, "public, max-age=3600")
            .header(CONTENT_TYPE, "text/html; charset=utf-8")
            .body(Body::from(pages::contact::page(&parsed_req)?.into_string()))?,

        Resource::Favicon => Response::builder()
            .header(LAST_MODIFIED, CONTACT_PAGE_LAST_MODIFIED)
            .header(CACHE_CONTROL, "public, max-age=31536000")
            .header(CONTENT_TYPE, "image/vnd.microsoft.icon")
            .body(Bytes::from_static(include_bytes!("favicon.ico")).into())?,

        Resource::Robots => Response::builder()
            .header(LAST_MODIFIED, ROBOTS_LAST_MODIFIED)
            .header(CACHE_CONTROL, "public, max-age=3600")
            .header(CONTENT_TYPE, "text/plain")
            .body(Bytes::from_static(include_bytes!("robots.txt")).into())?,
        Resource::BlockToB(block_hash) => {
            let network = network().as_url_path();
            Response::builder()
                .header(LOCATION, format!("{network}b/{block_hash}"))
                .status(StatusCode::TEMPORARY_REDIRECT)
                .body(Body::empty())?
        }
        Resource::TxToT(txid) => {
            let network = network().as_url_path();
            Response::builder()
                .header(LOCATION, format!("{network}t/{txid}"))
                .status(StatusCode::TEMPORARY_REDIRECT)
                .body(Body::empty())?
        }
        Resource::AddressToA(address) => {
            let network = network().as_url_path();
            Response::builder()
                .header(LOCATION, format!("{network}a/{address}"))
                .status(StatusCode::TEMPORARY_REDIRECT)
                .body(Body::empty())?
        }
        Resource::Address(ref address) => {
            if address.network != address_compatible(network()) {
                return Err(Error::AddressWrongNetwork {
                    address: address.network,
                    fbbe: network(),
                });
            } else {
                let page = pages::address::page(&address, &parsed_req)?.into_string();

                match parsed_req.response_type {
                    ResponseType::Text(col) => Response::builder()
                        .header(CONTENT_TYPE, TEXT_PLAIN_UTF_8.as_ref())
                        .body(pages::address::text_page(&address, &page, col)?.into())?,
                    ResponseType::Html => Response::builder()
                        .header(CONTENT_TYPE, TEXT_HTML_UTF_8.as_ref())
                        .body(page.into())?,
                    ResponseType::Bytes => {
                        return Err(Error::ContentTypeUnsupported(
                            parsed_req.response_type,
                            req.uri().to_string(),
                        ))
                    }
                }
            }
        }
        Resource::FullTx(ref tx) => {
            let txid = tx.txid();
            if state.tx(txid, false).await.is_ok() {
                let network = network().as_url_path();
                Response::builder()
                    .header(LOCATION, format!("{network}t/{txid}"))
                    .status(StatusCode::TEMPORARY_REDIRECT)
                    .body(Body::empty())?
            } else {
                let mempool_fees = state.mempool_fees.lock().await.clone();
                let prevouts = fetch_prevouts(tx, &state, true).await?;
                let page =
                    pages::tx::page(&tx, None, &prevouts, 0, mempool_fees, &parsed_req, true)?
                        .into_string();
                Response::builder()
                    .header(CONTENT_TYPE, TEXT_HTML_UTF_8.as_ref())
                    .body(page.into())?
            }
        } // parsed_req => {
          //     let page = format!("{:?}", parsed_req);
          //     Response::new(page.into())
          // }
    };

    log::debug!("{:?} executed in {:?}", req.uri(), now.elapsed());

    Ok(resp)
}

fn address_compatible(network: bitcoin::Network) -> bitcoin::Network {
    if let Network::Signet = network {
        Network::Testnet
    } else {
        network
    }
}

fn convert_text_html(page: &str, columns: usize) -> Body {
    convert_text_html_string(page, columns).into()
}

pub(crate) fn convert_text_html_string(page: &str, columns: usize) -> String {
    html2text::from_read_with_decorator(page.as_bytes(), columns, RichDecorator {})
}

fn cache_time_from_confirmations(confirmation: Option<u32>) -> u32 {
    match confirmation {
        None => 5,     // for txs, means it's unconfirmed
        Some(0) => 60, // means it's the block at the top
        Some(1) => 60 * 5,
        Some(2) => 60 * 30,
        Some(3) => 60 * 180,
        _ => 60 * 60 * 24 * 30, // one month
    }
}

async fn fetch_prevouts(
    tx: &bitcoin::Transaction,
    state: &SharedState,
    fill_missing: bool,
) -> Result<Vec<bitcoin::TxOut>, Error> {
    if tx.input.len() > 1 {
        state.preload_prevouts(tx).await;
    }
    let mut prevouts = Vec::with_capacity(tx.input.len());
    for input in tx.input.iter() {
        if input.previous_output.txid != Txid::all_zeros() {
            match state.tx(input.previous_output.txid, false).await {
                Ok((previous_tx, _)) => {
                    prevouts.push(previous_tx.output[input.previous_output.vout as usize].clone())
                }
                Err(e) => {
                    if fill_missing {
                        prevouts.push(TxOut::default())
                    } else {
                        return Err(e);
                    }
                }
            }
        } else {
            // fake txout for coinbase
            prevouts.push(TxOut::default())
        }
    }
    Ok(prevouts)
}

pub async fn route_infallible(
    req: Request<Body>,
    state: Arc<SharedState>,
) -> Result<Response<Body>, Infallible> {
    let resp = route(req, state).await.unwrap_or_else(|e| {
        let body = format!("{}", e);
        Response::builder()
            .status(StatusCode::from(e)) // TODO map errors to bad request or internal error
            .body(body.into())
            .expect("msg")
    });
    Ok(resp)
}
