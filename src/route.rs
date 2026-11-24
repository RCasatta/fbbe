use crate::{
    base_text_decorator::BaseTextDecorator,
    error::Error,
    network,
    pages::{self, tx::OutputStatus},
    render::{Io, MempoolSection, TxIoPath},
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
    header::{
        HeaderValue, CACHE_CONTROL, CONTENT_TYPE, IF_MODIFIED_SINCE, LAST_MODIFIED, LOCATION, VARY,
    },
    Request, Response, StatusCode,
};
use mime::{APPLICATION_OCTET_STREAM, TEXT_HTML_UTF_8, TEXT_PLAIN_UTF_8};
use prometheus::Encoder;
use std::{
    convert::Infallible,
    sync::Arc,
    time::{Duration, Instant},
};

const CSS_LAST_MODIFIED: &str = "2022-10-03 07:53:03 UTC";
const ABOUT_PAGE_LAST_MODIFIED: &str = "2022-12-16 07:53:03 UTC";
const FAVICON_LAST_MODIFIED: &str = "2024-03-25 08:25:36 UTC";
const ROBOTS_LAST_MODIFIED: &str = "2023-01-17 07:53:03 UTC";

#[derive(Debug, Clone, Copy)]
pub enum ResponseType {
    Text(u16),
    Html,
    Bytes,
    Hex,
}

impl ResponseType {
    pub fn is_text(&self) -> bool {
        matches!(self, ResponseType::Text(_))
    }

    /// The path suffix requesting this response type, used to keep it across redirects
    fn path_suffix(&self) -> &'static str {
        match self {
            ResponseType::Text(_) => "/text",
            ResponseType::Html => "",
            ResponseType::Bytes => "/bin",
            ResponseType::Hex => "/hex",
        }
    }
}

/// Redirect to `path` (relative to the network base path), keeping the requested response type
fn redirect(path: &str, response_type: ResponseType) -> Result<Response<Bytes>, Error> {
    let network = network().as_url_path();
    let suffix = response_type.path_suffix();
    let location = match path.split_once('#') {
        Some((path, fragment)) => format!("{network}{path}{suffix}#{fragment}"),
        None => format!("{network}{path}{suffix}"),
    };
    Ok(Response::builder()
        .header(LOCATION, location)
        .status(StatusCode::TEMPORARY_REDIRECT) // PERMANENT_REDIRECT cause issues in lynx
        .body(Bytes::new())?)
}

pub async fn route(
    req: Request<Bytes>,
    state: Arc<SharedState>,
    db: Option<Arc<Database>>,
) -> Result<Response<Bytes>, Error> {
    let parsed_req = req::parse(&req).await.map_err(Error::into_input_error)?;
    let vary = vary_header(&parsed_req);
    let mut resp = route_parsed(req, parsed_req, state, db).await?;
    if let Some(vary) = vary {
        resp.headers_mut()
            .insert(VARY, HeaderValue::from_static(vary));
    }
    Ok(resp)
}

/// Request headers other than the url changing the response, shared caches must keep apart
/// the text page from the html one, and text pages with different widths
fn vary_header(parsed_req: &req::ParsedRequest) -> Option<&'static str> {
    match (parsed_req.vary_accept, parsed_req.response_type.is_text()) {
        (true, true) => Some("Accept, columns"),
        (true, false) => Some("Accept"),
        (false, true) => Some("columns"),
        (false, false) => None,
    }
}

async fn route_parsed(
    req: Request<Bytes>,
    parsed_req: req::ParsedRequest,
    state: Arc<SharedState>,
    db: Option<Arc<Database>>,
) -> Result<Response<Bytes>, Error> {
    let now = Instant::now();
    // let _count = state.requests.fetch_add(1, Ordering::Relaxed);

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
            Resource::About => Some(ABOUT_PAGE_LAST_MODIFIED.to_string()),

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
                state.args.ads.as_deref(),
            )
            .into_string();

            let builder = Response::builder().header(CACHE_CONTROL, "public, max-age=5");
            match parsed_req.response_type {
                ResponseType::Text(col) => builder
                    .header(CONTENT_TYPE, TEXT_PLAIN_UTF_8.as_ref())
                    .body(convert_text_html(&page, col)?)?,
                ResponseType::Html => builder
                    .header(CONTENT_TYPE, TEXT_HTML_UTF_8.as_ref())
                    .body(Bytes::from(page))?,
                ResponseType::Bytes | ResponseType::Hex => {
                    return Err(Error::ContentTypeUnsupported(
                        parsed_req.response_type,
                        req.uri().to_string(),
                    ))
                }
            }
        }

        Resource::Block(block_hash, page) => {
            let block = rpc::block::call_json(block_hash).await?;
            let coinbase_txid = *block.tx.first().ok_or(Error::NotFound)?;
            let (coinbase, _) = state.tx(coinbase_txid, true).await?;
            let coinbase: bitcoin::Transaction =
                deserialize(coinbase.as_ref()).expect("invalid tx bytes");
            let coinbase_value = coinbase.output.iter().map(|o| o.value.to_sat()).sum();
            let page = pages::block::page(&block, page, coinbase_value, &parsed_req)?.into_string();
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
                    .body(convert_text_html(&page, col)?)?,
                ResponseType::Html => builder
                    .header(CONTENT_TYPE, TEXT_HTML_UTF_8.as_ref())
                    .body(Bytes::from(page))?,
                ResponseType::Bytes | ResponseType::Hex => {
                    return Err(Error::ContentTypeUnsupported(
                        parsed_req.response_type,
                        req.uri().to_string(),
                    ))
                }
            }
        }

        Resource::Tx(txid, pagination) => {
            if pagination > 0 {
                if let ResponseType::Bytes | ResponseType::Hex = parsed_req.response_type {
                    return Err(Error::BadRequest);
                }
            }
            let (ser_tx, block_hash) = state.tx(txid, true).await?;
            if let ResponseType::Hex = parsed_req.response_type {
                let current_tip = state.chain_info.lock().await.clone();
                let ts = match block_hash.as_ref() {
                    Some(block_hash) => Some(state.height_time(*block_hash).await?),
                    None => None,
                };
                let cache_seconds =
                    cache_time_from_confirmations(ts.map(|t| current_tip.blocks - t.height));
                return Ok(Response::builder()
                    .header(CACHE_CONTROL, format!("public, max-age={cache_seconds}"))
                    .header(CONTENT_TYPE, TEXT_PLAIN_UTF_8.as_ref())
                    .body(Bytes::from(ser_tx.0.to_lower_hex_string()))?);
            }
            let tx: bitcoin::Transaction = deserialize(ser_tx.as_ref()).expect("invalid tx bytes");
            let ts = match block_hash.as_ref() {
                Some(block_hash) => Some((*block_hash, state.height_time(*block_hash).await?)),
                None => None,
            };
            let (prevouts, fee) =
                tx_page_prevouts_and_fee(txid, &tx, &state, db.as_ref(), pagination).await?;
            let current_tip = state.chain_info.lock().await.clone();
            let mempool_fees = state.mempool_fees.lock().await.clone();
            let known_tx = state.known_txs.get(&txid).cloned();

            let output_status = output_status(&state, db, txid, tx.output.len()).await;
            let page = pages::tx::page(
                txid,
                &tx,
                ts,
                &prevouts,
                fee,
                output_status,
                pagination,
                mempool_fees,
                &parsed_req,
                false,
                known_tx,
            )?
            .into_string();
            let fee_still_computing = fee.is_none() && !tx.is_coinbase();
            let cache_seconds = if fee_still_computing {
                // the fee will be in the page once the background task caches it
                cache_time_from_confirmations(None)
            } else {
                cache_time_from_confirmations(ts.map(|t| current_tip.blocks - t.1.height))
            };

            let cache_control = format!("public, max-age={cache_seconds}");
            let mut builder = Response::builder().header(CACHE_CONTROL, cache_control);
            if let Some(ts) = ts {
                builder = builder.header(LAST_MODIFIED, ts.1.date_time_utc());
            }

            match parsed_req.response_type {
                ResponseType::Text(col) => builder
                    .header(CONTENT_TYPE, TEXT_PLAIN_UTF_8.as_ref())
                    .body(convert_text_html(&page, col)?)?,
                ResponseType::Html => builder
                    .header(CONTENT_TYPE, TEXT_HTML_UTF_8.as_ref())
                    .body(Bytes::from(page))?,
                ResponseType::Bytes => builder
                    .header(CONTENT_TYPE, APPLICATION_OCTET_STREAM.as_ref())
                    .body(Bytes::from(ser_tx.0))?,
                ResponseType::Hex => unreachable!("handled above"),
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
            let path = TxIoPath(&txid, Io::Input, vin).to_string();
            redirect(&path, parsed_req.response_type)?
        }

        Resource::SearchHeight(height) => {
            let hash = state
                .height_to_hash(height)
                .await
                .ok_or(Error::HeightNotFound(height))?;
            redirect(&format!("b/{hash}"), parsed_req.response_type)?
        }

        Resource::SearchBlock(hash) => redirect(&format!("b/{hash}"), parsed_req.response_type)?,

        Resource::SearchTx(txid) => redirect(&format!("t/{txid}"), parsed_req.response_type)?,

        Resource::SearchAddress(address) => {
            redirect(&format!("a/{address}"), parsed_req.response_type)?
        }

        Resource::Head => Response::new(Bytes::new()),

        Resource::Css => Response::builder()
            .header(LAST_MODIFIED, CSS_LAST_MODIFIED)
            .header(CACHE_CONTROL, "public, max-age=31536000")
            .header(CONTENT_TYPE, "text/css; charset=utf-8")
            .body(Bytes::from(include_str!("css/pico.min.css")))?,

        Resource::About => {
            let page = pages::about::page(&parsed_req)?.into_string();
            let builder = Response::builder()
                .header(LAST_MODIFIED, ABOUT_PAGE_LAST_MODIFIED)
                .header(CACHE_CONTROL, "public, max-age=3600");
            match parsed_req.response_type {
                ResponseType::Text(col) => builder
                    .header(CONTENT_TYPE, TEXT_PLAIN_UTF_8.as_ref())
                    .body(convert_text_html(&page, col)?)?,
                ResponseType::Html => builder
                    .header(CONTENT_TYPE, TEXT_HTML_UTF_8.as_ref())
                    .body(Bytes::from(page))?,
                ResponseType::Bytes | ResponseType::Hex => {
                    return Err(Error::ContentTypeUnsupported(
                        parsed_req.response_type,
                        req.uri().to_string(),
                    ))
                }
            }
        }

        Resource::Favicon => Response::builder()
            .header(LAST_MODIFIED, FAVICON_LAST_MODIFIED)
            .header(CACHE_CONTROL, "public, max-age=31536000")
            .header(CONTENT_TYPE, "image/vnd.microsoft.icon")
            .body(Bytes::from_static(include_bytes!("favicon.ico")))?,

        Resource::Robots => Response::builder()
            .header(LAST_MODIFIED, ROBOTS_LAST_MODIFIED)
            .header(CACHE_CONTROL, "public, max-age=3600")
            .header(CONTENT_TYPE, "text/plain")
            .body(Bytes::from_static(include_bytes!("robots.txt")))?,
        Resource::BlockToB(block_hash) => {
            redirect(&format!("b/{block_hash}"), parsed_req.response_type)?
        }
        Resource::TxToT(txid) => redirect(&format!("t/{txid}"), parsed_req.response_type)?,
        Resource::AddressToA(address) => {
            redirect(&format!("a/{address}"), parsed_req.response_type)?
        }
        Resource::Address(ref address, ref query) => {
            let address = address.clone().require_network(network())?;

            let address_outputs = match db {
                Some(db) => Some(address_seen(&address, db, state.clone()).await?),
                None => None,
            };
            let page =
                pages::address::page(&address, &parsed_req, query, address_outputs)?.into_string();
            let builder = Response::builder().header(CACHE_CONTROL, "public, max-age=60");

            match parsed_req.response_type {
                ResponseType::Text(col) => builder
                    .header(CONTENT_TYPE, TEXT_PLAIN_UTF_8.as_ref())
                    .body(Bytes::from(pages::address::text_page(&page, col)?))?,
                ResponseType::Html => builder
                    .header(CONTENT_TYPE, TEXT_HTML_UTF_8.as_ref())
                    .body(Bytes::from(page))?,
                ResponseType::Bytes | ResponseType::Hex => {
                    return Err(Error::ContentTypeUnsupported(
                        parsed_req.response_type,
                        req.uri().to_string(),
                    ))
                }
            }
        }
        Resource::SearchFullTx(ref tx) => {
            let txid = tx.compute_txid();

            if state.tx(txid, false).await.is_ok() {
                redirect(&format!("t/{txid}"), parsed_req.response_type)?
            } else {
                let bytes = serialize(&tx);
                let hex = bytes.to_lower_hex_string();

                redirect(&format!("txhex/{hex}"), parsed_req.response_type)?
            }
        }
        Resource::FullTx(ref tx) => {
            if let ResponseType::Hex = parsed_req.response_type {
                return Ok(Response::builder()
                    .header(CACHE_CONTROL, "public, max-age=3600")
                    .header(CONTENT_TYPE, TEXT_PLAIN_UTF_8.as_ref())
                    .body(Bytes::from(serialize(tx).to_lower_hex_string()))?);
            }
            let mempool_fees = state.mempool_fees.lock().await.clone();
            let txid = tx.compute_txid();
            let prevouts = fetch_prevouts(txid, tx, &state, true).await?;
            let output_status = output_status(&state, db, txid, tx.output.len()).await;

            let page = pages::tx::page(
                txid,
                tx,
                None,
                &prevouts,
                None,
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
                    .body(convert_text_html(&page, col)?)?,
                ResponseType::Html => builder
                    .header(CONTENT_TYPE, TEXT_HTML_UTF_8.as_ref())
                    .body(Bytes::from(page))?,
                ResponseType::Bytes => builder
                    .header(CONTENT_TYPE, APPLICATION_OCTET_STREAM.as_ref())
                    .body(Bytes::from(serialize(&tx)))?,
                ResponseType::Hex => unreachable!("handled above"),
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
        Resource::About => "About",
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
        ResponseType::Hex => "Hex",
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

fn convert_text_html(page: &str, columns: u16) -> Result<Bytes, Error> {
    Ok(Bytes::from(convert_text_html_string(page, columns)?))
}

pub(crate) fn convert_text_html_string(page: &str, columns: u16) -> Result<String, Error> {
    let page = strip_span_tags(page);
    let text = html2text::config::with_decorator(BaseTextDecorator {})
        .allow_width_overflow()
        .link_footnotes(true)
        .no_table_borders() // borders are mostly a 200 columns rule between rows, costly for LLMs
        .no_link_wrapping() // a footnote url split in multiple lines is unusable
        .min_wrap_width(min_wrap_width(columns))
        .string_from_read(page.as_bytes(), columns as usize)?;

    // table cells are padded with spaces, useless at the end of the line and costly for LLMs
    let mut result = String::with_capacity(text.len());
    for line in text.lines() {
        result.push_str(line.trim_end());
        result.push('\n');
    }
    Ok(result)
}

/// html2text sizes table columns proportionally to their content, a long value like a witness
/// squeezes the label column and wraps labels like "Previous script pubkey (v0 p2wpkh)".
/// A column is never narrower than the minimum wrap width (or its content), so a bigger one keeps
/// labels on a line; it scales with the width to keep narrow terminals tabular.
fn min_wrap_width(columns: u16) -> usize {
    (columns as usize / 5).clamp(12, 36) // 12 avoids wrapping amounts in narrow columns
}

/// Spans are used only for styling, in text mode they are useless and html2text undercounts
/// the width of inline elements in table cells, causing unneeded wrapping of hex strings.
fn strip_span_tags(page: &str) -> String {
    let mut result = String::with_capacity(page.len());
    let mut rest = page;
    while let Some(start) = [rest.find("<span"), rest.find("</span>")]
        .into_iter()
        .flatten()
        .min()
    {
        let Some(end) = rest[start..].find('>') else {
            break;
        };
        result.push_str(&rest[..start]);
        rest = &rest[start + end + 1..];
    }
    result.push_str(rest);
    result
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
        state.preload_prevouts(txid, tx, "tx_page").await;
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

async fn fetch_prevouts_page(
    txid: Txid,
    tx: &bitcoin::Transaction,
    state: &SharedState,
    page: usize,
) -> Result<Vec<bitcoin::TxOut>, Error> {
    let start = page * pages::tx::IO_PER_PAGE;
    if start >= tx.input.len() {
        return Ok(vec![TxOut::NULL; tx.input.len()]);
    }
    let end = (start + pages::tx::IO_PER_PAGE).min(tx.input.len());
    let mut prevouts = vec![TxOut::NULL; tx.input.len()];

    if end > start && end - start > 1 {
        state
            .preload_prevouts_inner(
                txid,
                tx.input[start..end].iter().map(|i| &i.previous_output),
                "tx_page",
            )
            .await;
    }

    for (i, input) in tx.input.iter().enumerate().skip(start).take(end - start) {
        if input.previous_output.txid != Txid::all_zeros() {
            let (previous_tx, _) = state.tx(input.previous_output.txid, false).await?;
            let tx_out = tx_output(previous_tx.as_ref(), input.previous_output.vout, true)
                .expect("invalid bytes");
            prevouts[i] = tx_out;
        }
    }

    Ok(prevouts)
}

async fn tx_page_prevouts_and_fee(
    txid: Txid,
    tx: &bitcoin::Transaction,
    state: &Arc<SharedState>,
    db: Option<&Arc<Database>>,
    page: usize,
) -> Result<(Vec<bitcoin::TxOut>, Option<u64>), Error> {
    const FEE_CACHE_INPUT_THRESHOLD: usize = 10;
    /// Waiting time for the fee of a large transaction, then the page is served without it
    const FEE_WAIT: Duration = Duration::from_secs(10);

    let db = match db {
        Some(db) if tx.input.len() > FEE_CACHE_INPUT_THRESHOLD && !tx.is_coinbase() => db,
        _ => {
            let prevouts = fetch_prevouts(txid, tx, state, false).await?;
            let fee = (!tx.is_coinbase()).then(|| fee(tx, &prevouts));
            return Ok((prevouts, fee));
        }
    };

    // For large transactions, a cached fee lets us fetch only the prevouts visible in the page.
    if let Some(fee) = db.get_fee(&txid) {
        let prevouts = fetch_prevouts_page(txid, tx, state, page).await?;
        return Ok((prevouts, Some(fee)));
    }

    // Fetching all the prevouts could take longer than the client or proxy timeout (eg. 20k
    // inputs), a dropped request would lose the work, so it runs in a task that caches the fee.
    if state.fee_in_flight.lock().await.insert(txid) {
        let (state, db, tx) = (state.clone(), db.clone(), tx.clone());
        let handle = tokio::spawn(async move {
            let result = fetch_prevouts(txid, &tx, &state, false).await;
            state.fee_in_flight.lock().await.remove(&txid);
            let prevouts = result?;
            let fee = fee(&tx, &prevouts);
            db.put_fee(&txid, fee)?;
            Ok::<_, Error>((prevouts, fee))
        });
        match tokio::time::timeout(FEE_WAIT, handle).await {
            Ok(Ok(result)) => return result.map(|(prevouts, fee)| (prevouts, Some(fee))),
            Ok(Err(join_error)) => return Err(Error::Join(join_error)),
            Err(_elapsed) => log::info!("fee of {txid} still computing after {FEE_WAIT:?}"),
        }
    }
    let prevouts = fetch_prevouts_page(txid, tx, state, page).await?;
    Ok((prevouts, None))
}

/// In satoshi, `prevouts` are all the outputs spent by the non-coinbase `tx`
fn fee(tx: &bitcoin::Transaction, prevouts: &[bitcoin::TxOut]) -> u64 {
    let sum_outputs: u64 = tx.output.iter().map(|o| o.value.to_sat()).sum();
    let sum_inputs: u64 = prevouts.iter().map(|o| o.value.to_sat()).sum();
    sum_inputs.saturating_sub(sum_outputs)
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
        let status = e.status_code();
        if status.is_server_error() {
            log::warn!("{e}");
        }
        Response::builder()
            .status(status)
            .header(CONTENT_TYPE, TEXT_PLAIN_UTF_8.as_ref())
            .body(Bytes::from(format!("{}\n", e.public_message())))
            .expect("msg")
    });

    timer.observe_duration();

    Ok(resp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn error_status_and_message() {
        let txid = Txid::all_zeros();
        let e = Error::RpcTxJson(StatusCode::NOT_FOUND, txid);
        assert_eq!(e.status_code(), StatusCode::NOT_FOUND);
        assert_eq!(e.public_message(), format!("Transaction {txid} not found"));

        let e = Error::RpcTxJson(StatusCode::SERVICE_UNAVAILABLE, txid);
        assert_eq!(e.status_code(), StatusCode::INTERNAL_SERVER_ERROR);

        let e = Error::from(Txid::from_str("nothex").unwrap_err()).into_input_error();
        assert_eq!(e.status_code(), StatusCode::BAD_REQUEST);
        assert!(e.public_message().starts_with("Bad request: "));

        let e = Error::NotFound.into_input_error();
        assert_eq!(e.status_code(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn strip_span_tags_keeps_content() {
        assert_eq!(
            strip_span_tags(r#"<code><span class="script">OP_0</span> ab</code><span>"#),
            "<code>OP_0 ab</code>"
        );
        assert_eq!(strip_span_tags("no spans"), "no spans");
    }

    #[test]
    fn text_table_does_not_wrap_script() {
        let html = r#"<table><tbody><tr><th>Type</th><td>p2wpkh</td></tr><tr><th>Script</th><td><code><span class="script">OP_0</span> <span class="script">OP_PUSHBYTES_20</span> e8df018c7e326cc253faac7e46cdc51e68542c42</code></td></tr></tbody></table>"#;
        let text = convert_text_html_string(html, 80).unwrap();
        assert!(text.contains("OP_0 OP_PUSHBYTES_20 e8df018c7e326cc253faac7e46cdc51e68542c42"));
        assert!(text.lines().all(|l| l == l.trim_end()));
    }

    #[test]
    fn text_label_does_not_wrap_next_to_long_value() {
        let long = "ab".repeat(90);
        let html = format!(
            "<table><tbody><tr><th>Previous script pubkey (v0 p2wpkh)</th><td><code>{long}</code></td></tr></tbody></table>"
        );
        let text = convert_text_html_string(&html, 200).unwrap();
        assert!(
            text.starts_with("Previous script pubkey (v0 p2wpkh) abab"),
            "{text}"
        );
    }

    #[test]
    fn text_footnotes_are_not_wrapped() {
        let link = format!("https://example.com/{}", "a".repeat(100));
        let html = format!(r#"<p><a href="{link}">link</a></p>"#);
        let text = convert_text_html_string(&html, 80).unwrap();
        assert!(text.contains(&format!("[1]: {link}\n")));
    }

    #[test]
    fn footnote_url_points_to_text() {
        use crate::base_text_decorator::footnote_url;
        let txid = "e5f65dd2b995966c74a5beec6a09934864fbdb64e7fb6b3b487e6bb9af2b236c";
        let host = Some("fbbe.info");
        assert_eq!(footnote_url("/", host), "https://fbbe.info/text");
        assert_eq!(footnote_url("/", None), "/text");
        assert_eq!(footnote_url("/signet/", None), "/signet/text");
        assert_eq!(
            footnote_url(&format!("/t/{txid}#o2"), host),
            format!("https://fbbe.info/t/{txid}/text#o2")
        );
        assert_eq!(
            footnote_url(&format!("/signet/t/{txid}/1#i12"), None),
            format!("/signet/t/{txid}/1/text#i12")
        );
        assert_eq!(
            footnote_url(&format!("/signet/b/{txid}/2"), None),
            format!("/signet/b/{txid}/2/text")
        );
        assert_eq!(footnote_url("/a/bc1q?q=1", None), "/a/bc1q/text?q=1");
        assert_eq!(
            footnote_url(&format!("/o/{txid}:0/100"), None),
            format!("/o/{txid}:0/100/text")
        );
        assert_eq!(
            footnote_url(&format!("/t/{txid}/hex"), None),
            format!("/t/{txid}/hex")
        );
        assert_eq!(footnote_url("/about", host), "https://fbbe.info/about/text");
        assert_eq!(
            footnote_url("https://github.com/x", host),
            "https://github.com/x"
        );
    }
}
