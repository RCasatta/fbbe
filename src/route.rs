use crate::{
    base_text_decorator::BaseTextDecorator,
    error::Error,
    network,
    pages::{self, tx::OutputStatus},
    render::MempoolSection,
    req::{self, Resource},
    rpc,
    state::tx_output,
    threads::index_addresses::{address_seen, Database},
    NetworkExt, SharedState,
};
use bitcoin::hex::DisplayHex;
use bitcoin::{consensus::serialize, OutPoint, TxOut, Txid};
use bitcoin::{
    consensus::{deserialize, Encodable},
    hashes::Hash,
};
use bitcoin_slices::{bsl, Visit, Visitor};
use hyper::body::Bytes;
use hyper::{
    header::{CACHE_CONTROL, CONTENT_TYPE, IF_MODIFIED_SINCE, LAST_MODIFIED, LOCATION},
    Request, Response, StatusCode,
};
use mime::{APPLICATION_OCTET_STREAM, TEXT_HTML_UTF_8, TEXT_PLAIN_UTF_8};
use prometheus::Encoder;
use std::{convert::Infallible, sync::Arc, time::Instant};

const CSS_LAST_MODIFIED: &str = "2022-10-03 07:53:03 UTC";
const CONTACT_PAGE_LAST_MODIFIED: &str = "2022-12-16 07:53:03 UTC";
const ROBOTS_LAST_MODIFIED: &str = "2023-01-17 07:53:03 UTC";

#[derive(Debug, Clone, Copy)]
pub enum ResponseType {
    Text(u16),
    Html,
    Bytes,
}

impl ResponseType {
    pub fn is_text(&self) -> bool {
        matches!(self, ResponseType::Text(_))
    }
}

pub async fn route(
    req: Request<Bytes>,
    state: Arc<SharedState>,
    db: Option<Arc<Database>>,
) -> Result<Response<Bytes>, Error> {
    let now = Instant::now();
    // let _count = state.requests.fetch_add(1, Ordering::Relaxed);
    let parsed_req = req::parse(&req).await?;

    handle_http_counter(&parsed_req);

    // DETERMINE IF NOT MODIFIED
    if let Some(if_modified_since) = req.headers().get(IF_MODIFIED_SINCE) {
        log::trace!("{:?} if modified since {:?}", req.uri(), if_modified_since);
        let modified = match &parsed_req.resource {
            // Resource::Tx(txid) => state.txs.lock().await.get(txid).map,
            Resource::Block(block_hash, _) => state
                .height_time(*block_hash)
                .await
                .ok()
                .map(|e| e.date_time_utc()),
            Resource::Tx(txid, _) => {
                if let Some(block_hash) = state.tx_in_block(txid).await {
                    state
                        .height_time(block_hash)
                        .await
                        .ok()
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
                    .body(Bytes::new())?);
            }
        }
    }

    let resp = match parsed_req.resource {
        Resource::Home => {
            let chain_info = state.chain_info.lock().await.clone();

            let mempool_section = MempoolSection {
                info: state.mempool_info.lock().await.clone(),
            };
            let fees = state.mempool_fees.lock().await.clone();

            let minute_since_blocks = state.minutes_since_block.lock().await.clone();
            let height_time = state.height_time(chain_info.best_block_hash).await?;
            let random_known_tx = if crate::network() == bitcoin::Network::Bitcoin {
                state.random_known_tx()
            } else {
                None
            };
            let page = pages::home::page(
                chain_info,
                height_time,
                mempool_section,
                minute_since_blocks,
                &parsed_req,
                fees,
                random_known_tx,
            )
            .into_string();

            let builder = Response::builder().header(CACHE_CONTROL, "public, max-age=5");
            match parsed_req.response_type {
                ResponseType::Text(col) => builder
                    .header(CONTENT_TYPE, TEXT_PLAIN_UTF_8.as_ref())
                    .body(convert_text_html(&page, col))?,
                ResponseType::Html => builder
                    .header(CONTENT_TYPE, TEXT_HTML_UTF_8.as_ref())
                    .body(Bytes::from(page))?,
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
                    .body(Bytes::from(page))?,
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
            let (ser_tx, block_hash) = state.tx(txid, true).await?;
            let tx: bitcoin::Transaction = deserialize(ser_tx.as_ref()).expect("invalid tx bytes");
            let ts = match block_hash.as_ref() {
                Some(block_hash) => Some((*block_hash, state.height_time(*block_hash).await?)),
                None => None,
            };
            let prevouts = fetch_prevouts(txid, &tx, &state, false).await?;
            let current_tip = state.chain_info.lock().await.clone();
            let mempool_fees = state.mempool_fees.lock().await.clone();
            let known_tx = state.known_txs.get(&txid).cloned();

            let output_status = output_status(&state, db, txid, tx.output.len()).await;
            let page = pages::tx::page(
                txid,
                &tx,
                ts,
                &prevouts,
                output_status,
                pagination,
                mempool_fees,
                &parsed_req,
                false,
                known_tx,
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
                    .body(Bytes::from(page))?,
                ResponseType::Bytes => builder
                    .header(CONTENT_TYPE, APPLICATION_OCTET_STREAM.as_ref())
                    .body(Bytes::from(ser_tx.0))?,
            }
        }

        Resource::TxOut(outpoint, height) => {
            let b = state.blocks_from_heights(&[height]).await?;
            struct FindTxByOutpointSpent(Vec<u8>, Option<(Txid, usize)>);
            impl Visitor for FindTxByOutpointSpent {
                fn visit_transaction(
                    &mut self,
                    tx: &bsl::Transaction,
                ) -> core::ops::ControlFlow<()> {
                    if let Some((txid, _vin)) = self.1.as_mut() {
                        *txid = tx.txid().into();
                        core::ops::ControlFlow::Break(())
                    } else {
                        core::ops::ControlFlow::Continue(())
                    }
                }

                fn visit_tx_in(
                    &mut self,
                    vin: usize,
                    tx_in: &bsl::TxIn,
                ) -> core::ops::ControlFlow<()> {
                    if tx_in.prevout().as_ref() == &self.0[..] {
                        self.1 = Some((Txid::all_zeros(), vin));
                    }
                    core::ops::ControlFlow::Continue(())
                }
            }
            let mut vec = Vec::with_capacity(36);
            outpoint.consensus_encode(&mut vec).unwrap();
            let mut visitor = FindTxByOutpointSpent(vec, None);
            let el = b.first().ok_or(Error::NotFound)?;
            match bsl::Block::visit(&el.1 .0, &mut visitor) {
                Ok(_) | Err(bitcoin_slices::Error::VisitBreak) => (),
                Err(_) => return Err(Error::NotFound), // TODO
            }

            let (txid, vin) = visitor.1.ok_or(Error::NotFound)?;
            let network = network().as_url_path();
            Response::builder()
                .header(LOCATION, format!("{network}t/{txid}#i{vin}"))
                .status(StatusCode::TEMPORARY_REDIRECT)
                .body(Bytes::new())?
        }

        Resource::SearchHeight(height) => {
            let hash = state
                .height_to_hash(height)
                .await
                .ok_or_else(|| Error::HeightNotFound)?;
            let network = network().as_url_path();

            Response::builder()
                .header(LOCATION, format!("{network}b/{hash}"))
                .status(StatusCode::TEMPORARY_REDIRECT) // PERMANENT_REDIRECT cause issues in lynx
                .body(Bytes::new())?
        }

        Resource::SearchBlock(hash) => {
            let network = network().as_url_path();
            Response::builder()
                .header(LOCATION, format!("{network}b/{hash}"))
                .status(StatusCode::TEMPORARY_REDIRECT) // PERMANENT_REDIRECT cause issues in lynx
                .body(Bytes::new())?
        }

        Resource::SearchTx(txid) => {
            let network = network().as_url_path();
            Response::builder()
                .header(LOCATION, format!("{network}t/{txid}"))
                .status(StatusCode::TEMPORARY_REDIRECT)
                .body(Bytes::new())?
        }

        Resource::SearchAddress(address) => {
            let network = network().as_url_path();
            Response::builder()
                .header(LOCATION, format!("{network}a/{address}"))
                .status(StatusCode::TEMPORARY_REDIRECT)
                .body(Bytes::new())?
        }

        Resource::Head => Response::new(Bytes::new()),

        Resource::Css => Response::builder()
            .header(LAST_MODIFIED, CSS_LAST_MODIFIED)
            .header(CACHE_CONTROL, "public, max-age=31536000")
            .header(CONTENT_TYPE, "text/css; charset=utf-8")
            .body(Bytes::from(include_str!("css/pico.min.css")))?,

        Resource::Contact => Response::builder()
            .header(LAST_MODIFIED, CONTACT_PAGE_LAST_MODIFIED)
            .header(CACHE_CONTROL, "public, max-age=3600")
            .header(CONTENT_TYPE, "text/html; charset=utf-8")
            .body(Bytes::from(
                pages::contact::page(&parsed_req)?.into_string(),
            ))?,

        Resource::Favicon => Response::builder()
            .header(LAST_MODIFIED, CONTACT_PAGE_LAST_MODIFIED)
            .header(CACHE_CONTROL, "public, max-age=31536000")
            .header(CONTENT_TYPE, "image/vnd.microsoft.icon")
            .body(Bytes::from_static(include_bytes!("favicon.ico")))?,

        Resource::Robots => Response::builder()
            .header(LAST_MODIFIED, ROBOTS_LAST_MODIFIED)
            .header(CACHE_CONTROL, "public, max-age=3600")
            .header(CONTENT_TYPE, "text/plain")
            .body(Bytes::from_static(include_bytes!("robots.txt")))?,
        Resource::BlockToB(block_hash) => {
            let network = network().as_url_path();
            Response::builder()
                .header(LOCATION, format!("{network}b/{block_hash}"))
                .status(StatusCode::TEMPORARY_REDIRECT)
                .body(Bytes::new())?
        }
        Resource::TxToT(txid) => {
            let network = network().as_url_path();
            Response::builder()
                .header(LOCATION, format!("{network}t/{txid}"))
                .status(StatusCode::TEMPORARY_REDIRECT)
                .body(Bytes::new())?
        }
        Resource::AddressToA(address) => {
            let network = network().as_url_path();
            Response::builder()
                .header(LOCATION, format!("{network}a/{address}"))
                .status(StatusCode::TEMPORARY_REDIRECT)
                .body(Bytes::new())?
        }
        Resource::Address(ref address, ref query) => {
            let address = address.clone().require_network(network())?;

            let address_seen = if let Some(db) = db {
                address_seen(&address, db, state.clone()).await?
            } else {
                vec![]
            };
            let page =
                pages::address::page(&address, &parsed_req, query, address_seen)?.into_string();
            let builder = Response::builder().header(CACHE_CONTROL, "public, max-age=60");

            match parsed_req.response_type {
                ResponseType::Text(col) => builder
                    .header(CONTENT_TYPE, TEXT_PLAIN_UTF_8.as_ref())
                    .body(Bytes::from(pages::address::text_page(
                        &address, &page, col,
                    )?))?,
                ResponseType::Html => builder
                    .header(CONTENT_TYPE, TEXT_HTML_UTF_8.as_ref())
                    .body(Bytes::from(page))?,
                ResponseType::Bytes => {
                    return Err(Error::ContentTypeUnsupported(
                        parsed_req.response_type,
                        req.uri().to_string(),
                    ))
                }
            }
        }
        Resource::SearchFullTx(ref tx) => {
            let txid = tx.compute_txid();
            let network = network().as_url_path();

            if state.tx(txid, false).await.is_ok() {
                Response::builder()
                    .header(LOCATION, format!("{network}t/{txid}"))
                    .status(StatusCode::TEMPORARY_REDIRECT)
                    .body(Bytes::new())?
            } else {
                let bytes = serialize(&tx);
                let hex = bytes.to_lower_hex_string();

                Response::builder()
                    .header(LOCATION, format!("{network}txhex/{hex}"))
                    .status(StatusCode::TEMPORARY_REDIRECT)
                    .body(Bytes::new())?
            }
        }
        Resource::FullTx(ref tx) => {
            let mempool_fees = state.mempool_fees.lock().await.clone();
            let txid = tx.compute_txid();
            let prevouts = fetch_prevouts(txid, tx, &state, true).await?;
            let output_status = output_status(&state, db, txid, tx.output.len()).await;

            let page = pages::tx::page(
                txid,
                tx,
                None,
                &prevouts,
                output_status,
                0,
                mempool_fees,
                &parsed_req,
                true,
                None,
            )?
            .into_string();
            let builder = Response::builder().header(CACHE_CONTROL, "public, max-age=3600");

            match parsed_req.response_type {
                ResponseType::Text(col) => builder
                    .header(CONTENT_TYPE, TEXT_PLAIN_UTF_8.as_ref())
                    .body(convert_text_html(&page, col))?,
                ResponseType::Html => builder
                    .header(CONTENT_TYPE, TEXT_HTML_UTF_8.as_ref())
                    .body(Bytes::from(page))?,
                ResponseType::Bytes => builder
                    .header(CONTENT_TYPE, APPLICATION_OCTET_STREAM.as_ref())
                    .body(Bytes::from(serialize(&tx)))?,
            }
        }
        Resource::Metrics => {
            let encoder = prometheus::TextEncoder::new();

            let metric_families = prometheus::gather();
            let mut buffer = vec![];
            encoder.encode(&metric_families, &mut buffer)?;
            Response::builder()
                .status(200)
                .header(CONTENT_TYPE, encoder.format_type())
                .body(Bytes::from(buffer))?
        }
        Resource::Sitemap => {
            let dns_host = match state.args.dns_host.as_ref() {
                Some(dns_host) => dns_host,
                None => {
                    return Err(Error::NotFound);
                }
            };

            // Build the XML sitemap
            // TODO build once and put in the state.
            let mut sitemap = String::from(
                r#"<?xml version="1.0" encoding="UTF-8"?><urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">"#,
            );

            // Add home page
            sitemap.push_str(&format!(
                "<url><loc>https://{}/</loc><changefreq>always</changefreq><priority>1.0</priority></url>",
                dns_host
            ));

            // Add known transactions from state
            for txid in state.known_txs.keys() {
                sitemap.push_str(&format!(
                    "<url><loc>https://{}/t/{}</loc><changefreq>never</changefreq><priority>0.8</priority></url>",
                    dns_host, txid
                ));
            }

            sitemap.push_str("\n</urlset>");

            Response::builder()
                .header(CONTENT_TYPE, "application/xml; charset=utf-8")
                .header(CACHE_CONTROL, "public, max-age=86400") // Cache for 24 hours
                .body(Bytes::from(sitemap))?
        }
    };

    log::debug!("{:?} executed in {:?}", req.uri(), now.elapsed());

    Ok(resp)
}

fn handle_http_counter(parsed_req: &req::ParsedRequest) {
    let resource = match &parsed_req.resource {
        Resource::Home => "Home",
        Resource::Favicon => "Favicon",
        Resource::Css => "Css",
        Resource::Contact => "Contact",
        Resource::SearchHeight(_) => "SearchHeight",
        Resource::SearchBlock(_) => "SearchBlock",
        Resource::SearchTx(_) => "SearchTx",
        Resource::SearchAddress(_) => "SearchAddress",
        Resource::SearchFullTx(_) => "SearchFullTx",
        Resource::Tx(_, _) => "Tx",
        Resource::Block(_, _) => "Block",
        Resource::TxOut(_, _) => "TxOut",
        Resource::Head => "Head",
        Resource::Robots => "Robots",
        Resource::BlockToB(_) => "BlockToB",
        Resource::TxToT(_) => "TxToT",
        Resource::Address(_, _) => "Address",
        Resource::AddressToA(_) => "AddressToA",
        Resource::FullTx(_) => "FullTx",
        Resource::Metrics => "Metrics",
        Resource::Sitemap => "Sitemap",
    };
    let content = match &parsed_req.response_type {
        ResponseType::Text(_) => "Text",
        ResponseType::Html => "Html",
        ResponseType::Bytes => "Bytes",
    };
    crate::HTTP_COUNTER
        .with_label_values(&[resource, content])
        .inc();
}

async fn output_status(
    state: &Arc<SharedState>,
    db: Option<Arc<Database>>,
    txid: Txid,
    len: usize,
) -> Vec<OutputStatus> {
    let mut result = Vec::with_capacity(len);
    for i in 0..len {
        let k = OutPoint::new(txid, i as u32);
        let r = match state.mempool_spending.lock().await.get(&k).cloned() {
            Some(v) => OutputStatus::UnconfirmedSpent(v),
            None => {
                match db.as_ref() {
                    Some(db) => {
                        // TODO use iteration
                        let outpoint = OutPoint::new(txid, i as u32);
                        if let Some(res) = db.get_spending(&outpoint) {
                            OutputStatus::ConfirmedSpent(res)
                        } else {
                            OutputStatus::Unspent
                        }
                    }
                    None => OutputStatus::Unknown,
                }
            }
        };
        result.push(r);
    }
    result
}

fn convert_text_html(page: &str, columns: u16) -> Bytes {
    Bytes::from(convert_text_html_string(page, columns))
}

pub(crate) fn convert_text_html_string(page: &str, columns: u16) -> String {
    html2text::from_read_with_decorator(page.as_bytes(), columns as usize, BaseTextDecorator {})
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

pub async fn fetch_prevouts(
    txid: Txid,
    tx: &bitcoin::Transaction,
    state: &SharedState,
    fill_missing: bool,
) -> Result<Vec<bitcoin::TxOut>, Error> {
    if tx.input.len() > 1 {
        state.preload_prevouts(txid, tx).await;
    }
    let mut prevouts = Vec::with_capacity(tx.input.len());
    for input in tx.input.iter() {
        if input.previous_output.txid != Txid::all_zeros() {
            match state.tx(input.previous_output.txid, false).await {
                Ok((previous_tx, _)) => {
                    let tx_out = tx_output(previous_tx.as_ref(), input.previous_output.vout, true)
                        .expect("invalid bytes");
                    prevouts.push(tx_out);
                }
                Err(e) => {
                    if fill_missing {
                        prevouts.push(TxOut::NULL)
                    } else {
                        return Err(e);
                    }
                }
            }
        } else {
            // fake txout for coinbase
            prevouts.push(TxOut::NULL)
        }
    }
    Ok(prevouts)
}

pub async fn route_infallible(
    req: Request<Bytes>,
    state: Arc<SharedState>,
    db: Option<Arc<Database>>,
) -> Result<Response<Bytes>, Infallible> {
    let timer = crate::HTTP_REQ_HISTOGRAM
        .with_label_values(&["all"])
        .start_timer();

    let resp = route(req, state, db).await.unwrap_or_else(|e| {
        let body = format!("{}", e);
        Response::builder()
            .status(StatusCode::from(e)) // TODO map errors to bad request or internal error
            .body(Bytes::from(body))
            .expect("msg")
    });

    timer.observe_duration();

    Ok(resp)
}
