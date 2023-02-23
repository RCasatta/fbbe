use crate::{
    error::Error,
    network, pages,
    render::MempoolSection,
    req::{self, ParsedRequest},
    rpc, NetworkExt, SharedState,
};
use bitcoin::{consensus::serialize, OutPoint, TxOut, Txid};
use bitcoin_hashes::Hash;
use html2text::render::text_renderer::RichDecorator;
use hyper::{
    body::Bytes,
    header::{
        ACCEPT, CACHE_CONTROL, CONTENT_TYPE, IF_MODIFIED_SINCE, LAST_MODIFIED, LOCATION,
        USER_AGENT, VARY,
    },
    Body, Request, Response, StatusCode,
};
use mime::{
    APPLICATION_OCTET_STREAM, OCTET_STREAM, STAR_STAR, TEXT_HTML_UTF_8, TEXT_PLAIN,
    TEXT_PLAIN_UTF_8,
};
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
    log::debug!("request: {:?}", req);
    // let _count = state.requests.fetch_add(1, Ordering::Relaxed);
    let parsed_req = req::parse(&req).await?;
    let response_type = parse_response_content_type(&req);
    log::debug!("response in: {:?}", response_type);

    // DETERMINE IF NOT MODIFIED
    if let Some(if_modified_since) = req.headers().get(IF_MODIFIED_SINCE) {
        log::trace!("{:?} if modified since {:?}", req.uri(), if_modified_since);
        let modified = match &parsed_req {
            // ParsedRequest::Tx(txid) => state.txs.lock().await.get(txid).map,
            ParsedRequest::Block(block_hash, _) => state
                .hash_to_height_time
                .lock()
                .await
                .get(block_hash)
                .map(|e| e.date_time_utc()),
            ParsedRequest::Tx(txid, _) => {
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
            ParsedRequest::Css => Some(CSS_LAST_MODIFIED.to_string()),
            ParsedRequest::Contact => Some(CONTACT_PAGE_LAST_MODIFIED.to_string()),

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

    let resp = match parsed_req {
        ParsedRequest::Home => {
            let chain_info = state.chain_info.lock().await.clone();
            let info = state.mempool_info.lock().await.clone();
            let fees = state.mempool_fees.lock().await.clone();
            let mempool_section = MempoolSection { info, fees };

            let height_time = state.height_time(chain_info.best_block_hash).await?;
            let page = pages::home::page(chain_info, height_time, mempool_section, response_type)
                .into_string();

            let builder = Response::builder().header(CACHE_CONTROL, "public, max-age=5");
            match response_type {
                ResponseType::Text(col) => builder
                    .header(CONTENT_TYPE, TEXT_PLAIN_UTF_8.as_ref())
                    .header(VARY, "Accept")
                    .body(convert_text_html(page, col))?,
                ResponseType::Html => builder
                    .header(CONTENT_TYPE, TEXT_HTML_UTF_8.as_ref())
                    .header(VARY, "Accept")
                    .body(page.into())?,
                ResponseType::Bytes => {
                    return Err(Error::ContentTypeUnsupported(
                        response_type,
                        req.uri().to_string(),
                    ))
                }
            }
        }

        ParsedRequest::Block(block_hash, page) => {
            let block = rpc::block::call_json(block_hash).await?;
            let page = pages::block::page(&block, page, response_type)?.into_string();
            let current_tip = state.chain_info.lock().await.clone();
            let block_confirmations = current_tip.blocks - block.height;
            let cache_seconds = cache_time_from_confirmations(Some(block_confirmations));
            let cache_control = format!("public, max-age={cache_seconds}");

            let builder = Response::builder()
                .header(CACHE_CONTROL, cache_control) // cache examples https://developers.cloudflare.com/cache/about/cache-control/#examples
                .header(LAST_MODIFIED, block.date_time_utc());

            match response_type {
                ResponseType::Text(col) => builder
                    .header(CONTENT_TYPE, TEXT_PLAIN_UTF_8.as_ref())
                    .header(VARY, "Accept")
                    .body(convert_text_html(page, col))?,
                ResponseType::Html => builder
                    .header(CONTENT_TYPE, TEXT_HTML_UTF_8.as_ref())
                    .header(VARY, "Accept")
                    .body(page.into())?,
                ResponseType::Bytes => {
                    return Err(Error::ContentTypeUnsupported(
                        response_type,
                        req.uri().to_string(),
                    ))
                }
            }
        }

        ParsedRequest::Tx(txid, pagination) => {
            let (tx, block_hash) = state.tx(txid, true).await?;
            let ts = match block_hash.as_ref() {
                Some(block_hash) => Some((*block_hash, state.height_time(*block_hash).await?)),
                None => None,
            };
            let prevouts = fetch_prevouts(&tx, &state).await?;
            let current_tip = state.chain_info.lock().await.clone();
            let mempool_fees = state.mempool_fees.lock().await.clone();
            let page =
                pages::tx::page(&tx, ts, &prevouts, pagination, mempool_fees, response_type)?
                    .into_string();
            let cache_seconds =
                cache_time_from_confirmations(ts.map(|t| current_tip.blocks - t.1.height));

            let cache_control = format!("public, max-age={cache_seconds}");
            let mut builder = Response::builder().header(CACHE_CONTROL, cache_control);
            if let Some(ts) = ts {
                builder = builder.header(LAST_MODIFIED, ts.1.date_time_utc());
            }

            match response_type {
                ResponseType::Text(col) => builder
                    .header(CONTENT_TYPE, TEXT_PLAIN_UTF_8.as_ref())
                    .header(VARY, "Accept")
                    .body(convert_text_html(page, col))?,
                ResponseType::Html => builder
                    .header(CONTENT_TYPE, TEXT_HTML_UTF_8.as_ref())
                    .header(VARY, "Accept")
                    .body(page.into())?,
                ResponseType::Bytes => builder
                    .header(CONTENT_TYPE, APPLICATION_OCTET_STREAM.as_ref())
                    .header(VARY, "Accept")
                    .body(Bytes::from(serialize(&tx)).into())?,
            }
        }

        ParsedRequest::TxOut(txid, vout) => match rpc::txout::call(txid, vout).await {
            Ok(tx) => {
                let outpoint = OutPoint::new(txid, vout);
                let page = pages::txout::page(&tx, outpoint, response_type).into_string();
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

        ParsedRequest::SearchHeight(height) => {
            let hash = state.hash(height).await?;
            let network = network().as_url_path();

            Response::builder()
                .header(LOCATION, format!("{network}b/{hash}"))
                .status(StatusCode::TEMPORARY_REDIRECT) // PERMANENT_REDIRECT cause issues in lynx
                .body(Body::empty())?
        }

        ParsedRequest::SearchBlock(hash) => {
            let network = network().as_url_path();
            Response::builder()
                .header(LOCATION, format!("{network}b/{hash}"))
                .status(StatusCode::TEMPORARY_REDIRECT) // PERMANENT_REDIRECT cause issues in lynx
                .body(Body::empty())?
        }

        ParsedRequest::SearchTx(txid) => {
            let network = network().as_url_path();
            Response::builder()
                .header(LOCATION, format!("{network}t/{txid}"))
                .status(StatusCode::TEMPORARY_REDIRECT)
                .body(Body::empty())?
        }

        ParsedRequest::SearchAddress(address) => {
            let network = network().as_url_path();
            Response::builder()
                .header(LOCATION, format!("{network}a/{address}"))
                .status(StatusCode::TEMPORARY_REDIRECT)
                .body(Body::empty())?
        }

        ParsedRequest::Head => Response::new(Body::empty()),

        ParsedRequest::Css => Response::builder()
            .header(LAST_MODIFIED, CSS_LAST_MODIFIED)
            .header(CACHE_CONTROL, "public, max-age=31536000")
            .header(CONTENT_TYPE, "text/css; charset=utf-8")
            .body(Body::from(include_str!("css/pico.min.css")))?,

        ParsedRequest::Contact => Response::builder()
            .header(LAST_MODIFIED, CONTACT_PAGE_LAST_MODIFIED)
            .header(CACHE_CONTROL, "public, max-age=3600")
            .header(CONTENT_TYPE, "text/html; charset=utf-8")
            .body(Body::from(
                pages::contact::page(response_type)?.into_string(),
            ))?,

        ParsedRequest::Favicon => Response::builder()
            .header(LAST_MODIFIED, CONTACT_PAGE_LAST_MODIFIED)
            .header(CACHE_CONTROL, "public, max-age=31536000")
            .header(CONTENT_TYPE, "image/vnd.microsoft.icon")
            .body(Bytes::from_static(include_bytes!("favicon.ico")).into())?,

        ParsedRequest::Robots => Response::builder()
            .header(LAST_MODIFIED, ROBOTS_LAST_MODIFIED)
            .header(CACHE_CONTROL, "public, max-age=3600")
            .header(CONTENT_TYPE, "text/plain")
            .body(Bytes::from_static(include_bytes!("robots.txt")).into())?,
        ParsedRequest::BlockToB(block_hash) => {
            let network = network().as_url_path();
            Response::builder()
                .header(LOCATION, format!("{network}b/{block_hash}"))
                .status(StatusCode::TEMPORARY_REDIRECT)
                .body(Body::empty())?
        }
        ParsedRequest::TxToT(txid) => {
            let network = network().as_url_path();
            Response::builder()
                .header(LOCATION, format!("{network}t/{txid}"))
                .status(StatusCode::TEMPORARY_REDIRECT)
                .body(Body::empty())?
        }
        ParsedRequest::AddressToA(address) => {
            let network = network().as_url_path();
            Response::builder()
                .header(LOCATION, format!("{network}a/{address}"))
                .status(StatusCode::TEMPORARY_REDIRECT)
                .body(Body::empty())?
        }
        ParsedRequest::Address(address) => {
            if address.network != network() {
                return Err(Error::AddressWrongNetwork {
                    address: address.network,
                    fbbe: network(),
                });
            } else {
                let page = pages::address::page(&address, response_type)?.into_string();

                Response::builder()
                    .header(CACHE_CONTROL, "public, max-age=5")
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

fn convert_text_html(page: String, columns: usize) -> Body {
    html2text::from_read_with_decorator(&page.into_bytes()[..], columns, RichDecorator {}).into()
}

fn parse_response_content_type(req: &Request<Body>) -> ResponseType {
    match req.headers().get(ACCEPT) {
        None => ResponseType::Html,
        Some(accept) => {
            if accept == TEXT_PLAIN_UTF_8.as_ref() || accept == TEXT_PLAIN.as_ref() {
                ResponseType::Text(parse_cols(&req))
            } else if accept == OCTET_STREAM.as_ref() || accept == APPLICATION_OCTET_STREAM.as_ref()
            {
                ResponseType::Bytes
            } else if accept == STAR_STAR.as_ref() {
                if req
                    .headers()
                    .get(USER_AGENT)
                    .map(|e| e.to_str().ok())
                    .flatten()
                    .map(|e| e.starts_with("curl"))
                    .unwrap_or(false)
                {
                    ResponseType::Text(parse_cols(req))
                } else {
                    ResponseType::Html
                }
            } else {
                ResponseType::Html
            }
        }
    }
}

fn parse_cols(req: &Request<Body>) -> usize {
    req.headers()
        .get("columns")
        .and_then(|c| c.to_str().ok())
        .and_then(|e| e.parse::<usize>().ok())
        .unwrap_or(80)
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
) -> Result<Vec<bitcoin::TxOut>, Error> {
    if tx.input.len() > 1 {
        state.preload_prevouts(tx).await;
    }
    let mut prevouts = Vec::with_capacity(tx.input.len());
    for input in tx.input.iter() {
        if input.previous_output.txid != Txid::all_zeros() {
            let previous_tx = state.tx(input.previous_output.txid, false).await?.0;
            prevouts.push(previous_tx.output[input.previous_output.vout as usize].clone());
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
