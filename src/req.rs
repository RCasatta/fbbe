use std::fmt::Display;
use std::str::FromStr;

use crate::globals::network;
use crate::threads::index_addresses::Height;
use crate::NetworkExt;
use crate::{error::Error, route::ResponseType};
use bitcoin::address::NetworkUnchecked;
use bitcoin::hashes::{sha256d, Hash};
use bitcoin::{consensus::deserialize, Address, BlockHash, Transaction, Txid};
use bitcoin::{OutPoint, Psbt};
use hyper::{header::ACCEPT, Method, Request};

#[derive(Debug, Clone)]
pub struct ParsedRequest {
    pub resource: Resource,
    pub response_type: ResponseType,
    /// The response type is not in the path, so it depends on the `Accept` header
    pub vary_accept: bool,
}

#[derive(Debug, Clone)]
pub enum Resource {
    Home,
    Favicon,
    Css,
    About,
    SearchHeight(u32),
    SearchBlock(BlockHash),
    SearchTx(Txid),
    SearchAddress(Address),
    SearchFullTx(Transaction),
    Tx(Txid, usize),
    Block(BlockHash, usize),
    TxOut(OutPoint, Height),
    Head,
    Robots,
    BlockToB(BlockHash),
    TxToT(Txid),
    Address(Address<NetworkUnchecked>, Option<String>, usize),
    AddressToA(Address),
    FullTx(Transaction),
    Metrics,
    Sitemap,
}

pub async fn parse(req: &Request<hyper::body::Bytes>) -> Result<ParsedRequest, Error> {
    let mut path: Vec<_> = req.uri().path().split('/').skip(1).take(5).collect();
    log::debug!("{:?}", path);

    if path.get(4).is_some() {
        return Err(Error::BadRequest);
    }
    let (response_type, vary_accept) = match path.last() {
        Some(&"text") => (ResponseType::Text(parse_cols(req)), false),
        Some(&"bin") => (ResponseType::Bytes, false),
        Some(&"hex") => (ResponseType::Hex, false),
        _ if accepts_text_not_html(req) => (ResponseType::Text(parse_cols(req)), true),
        _ => (ResponseType::Html, true),
    };
    log::debug!("{:?}", response_type);
    if !vary_accept {
        path.pop();
        if path.is_empty() {
            // home page corner case
            path.push("");
        }
    }
    let query = req.uri().query();
    let is_head = req.method() == Method::HEAD;
    let method = if is_head { &Method::GET } else { req.method() };

    let mut resource = match (method, query, path.first(), path.get(1), path.get(2)) {
        (&Method::GET, None, Some(&""), None, None) => Resource::Home,
        (&Method::GET, Some(query), None | Some(&""), None, None) => {
            if query.contains('&') {
                return Err(Error::BadRequest);
            }
            let mut iter = query.split('=');
            if iter.next() != Some("s") {
                return Err(Error::BadRequest);
            }
            match (iter.next(), iter.next()) {
                (Some(val), None) => match val.parse::<u32>() {
                    Ok(height) => Resource::SearchHeight(height),
                    Err(_) => match sha256d::Hash::from_str(val) {
                        Ok(val) => {
                            if val.to_byte_array().ends_with(&[0u8; 4]) {
                                Resource::SearchBlock(val.into())
                            } else {
                                Resource::SearchTx(val.into())
                            }
                        }
                        Err(_) => match Address::from_str(val) {
                            Ok(address) => Resource::SearchAddress(address.assume_checked()),
                            Err(_) => {
                                match hex::decode(val)
                                    .map(|bytes| deserialize::<Transaction>(&bytes))
                                {
                                    Ok(Ok(tx)) => Resource::SearchFullTx(tx),
                                    _ => {
                                        let val = percent_encoding::percent_decode(val.as_bytes())
                                            .decode_utf8()
                                            .map_err(|_| search_error())?;
                                        let psbt = Psbt::from_str(val.as_ref())
                                            .map_err(|_| search_error())?;
                                        let tx = psbt.extract_tx()?;
                                        Resource::SearchFullTx(tx)
                                    }
                                }
                            }
                        },
                    },
                },
                _ => return Err(Error::BadRequest),
            }
        }

        (&Method::GET, None, Some(&"favicon.ico"), None, None) => Resource::Favicon,
        (&Method::GET, None, Some(&"robots.txt"), None, None) => Resource::Robots,
        (&Method::GET, None, Some(&"css"), Some(&"pico.min.css"), None) => Resource::Css,
        (&Method::GET, None, Some(&"about"), None, None) => Resource::About,
        (&Method::GET, None, Some(&"metrics"), None, None) => Resource::Metrics,
        (&Method::GET, None, Some(&"sitemap.xml"), None, None) => Resource::Sitemap,

        (&Method::GET, _, Some(&"t"), Some(txid), page) => {
            let txid = Txid::from_str(txid)?;
            let page = match page {
                Some(page) => page.parse::<usize>()?,
                None => 0,
            };
            Resource::Tx(txid, page)
        }
        (&Method::GET, None, Some(&"o"), Some(outpoint), Some(height)) => {
            let outpoint = OutPoint::from_str(outpoint)?;
            let height: u32 = height.parse()?;
            Resource::TxOut(outpoint, height)
        }
        (&Method::GET, None, Some(&"h"), Some(height), None) => {
            let height: u32 = height.parse()?;
            Resource::SearchHeight(height)
        }
        (&Method::GET, None, Some(&"b"), Some(block_hash), page) => {
            let block_hash = BlockHash::from_str(block_hash)?;
            let page = match page {
                Some(page) => page.parse::<usize>()?,
                None => 0,
            };
            Resource::Block(block_hash, page)
        }
        (&Method::GET, query, Some(&"a"), Some(address), page) => {
            let address = parse_address(address)?;
            let page = match page {
                Some(page) => page.parse::<usize>()?,
                None => 0,
            };
            Resource::Address(address, query.map(ToString::to_string), page)
        }
        (&Method::GET, None, Some(&"block"), Some(block_hash), None) => {
            let block_hash = BlockHash::from_str(block_hash)?;
            Resource::BlockToB(block_hash)
        }
        (&Method::GET, _, Some(&"tx"), Some(txid), None) => {
            let txid = Txid::from_str(txid)?;
            Resource::TxToT(txid)
        }
        (&Method::GET, None, Some(&"txhex"), Some(hex), None) => {
            let bytes = hex::decode(hex)?;
            let tx: Transaction = deserialize(&bytes)?;
            Resource::FullTx(tx)
        }
        (&Method::GET, None, Some(&"address"), Some(address), None) => {
            let address = parse_address(address)?;
            Resource::AddressToA(address.assume_checked())
        }
        _ => return Err(Error::NotFound),
    };

    if is_head {
        resource = Resource::Head;
    }
    Ok(ParsedRequest {
        resource,
        response_type,
        vary_accept,
    })
}

fn search_error() -> Error {
    Error::InvalidInput(
        "search for a txid, block hash, height, address, raw transaction hex or PSBT".to_owned(),
    )
}

/// The parse error of an invalid bech32 address is reported as base58, which is misleading
fn parse_address(address: &str) -> Result<Address<NetworkUnchecked>, Error> {
    Address::from_str(address)
        .map_err(|_| Error::InvalidInput(format!("invalid address {address}")))
}

/// True when the client prefers `text/plain` or `text/markdown` over html, as agents do: they may
/// list `text/html` with a lower `q`, browsers always rank it first. Ties go to html.
/// The text page reads well as markdown, so it serves both.
fn accepts_text_not_html(req: &Request<hyper::body::Bytes>) -> bool {
    let Some(accept) = req.headers().get(ACCEPT).and_then(|a| a.to_str().ok()) else {
        return false;
    };
    let mut text_q = 0.0f32;
    let mut html_q = 0.0f32;
    for media_range in accept.split(',') {
        let mut params = media_range.split(';');
        let media_type = params
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        let q = params
            .filter_map(|p| p.trim().strip_prefix("q="))
            .find_map(|q| q.trim().parse::<f32>().ok())
            .unwrap_or(1.0);
        match media_type.as_str() {
            "text/html" | "application/xhtml+xml" => html_q = html_q.max(q),
            "text/plain" | "text/markdown" => text_q = text_q.max(q),
            _ => (),
        }
    }
    text_q > html_q
}

pub struct TextLink<'a>(&'a Resource);
impl Display for TextLink<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let base = network().as_url_path();
        match self.0 {
            Resource::Home => write!(f, "{}text", base),

            Resource::Tx(txid, pagination) => {
                if *pagination == 0 {
                    write!(f, "{base}t/{txid}/text")
                } else {
                    write!(f, "{base}t/{txid}/{pagination}/text")
                }
            }
            Resource::Block(block_hash, pagination) => {
                if *pagination == 0 {
                    write!(f, "{base}b/{block_hash}/text")
                } else {
                    write!(f, "{base}b/{block_hash}/{pagination}/text")
                }
            }
            Resource::Address(address, query, pagination) => {
                let address = address.clone().assume_checked(); // TODO clone is a performance penalty here
                if *pagination == 0 {
                    write!(f, "{base}a/{address}/text")?;
                } else {
                    write!(f, "{base}a/{address}/{pagination}/text")?;
                }
                if let Some(query) = query {
                    write!(f, "?{query}")?;
                }
                Ok(())
            }
            _ => panic!("resource without text link"),
        }
    }
}
impl Resource {
    pub fn link(&self) -> Option<TextLink> {
        use Resource::*;
        match self {
            Home | Tx(_, _) | Block(_, _) | Address(_, _, _) => Some(TextLink(self)),
            _ => None,
        }
    }
}

fn parse_cols(req: &Request<hyper::body::Bytes>) -> u16 {
    req.headers()
        .get("columns")
        .and_then(|c| c.to_str().ok())
        .and_then(|e| e.parse::<u16>().ok())
        .unwrap_or(200)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyper::body::Bytes;

    fn request_accept(uri: &str, accept: &str) -> Request<Bytes> {
        Request::builder()
            .uri(uri)
            .header(ACCEPT, accept)
            .body(Bytes::new())
            .expect("valid request")
    }

    #[tokio::test]
    async fn parse_accept_header() {
        async fn text(accept: &str) -> bool {
            let parsed = parse(&request_accept("/", accept)).await.unwrap();
            assert!(parsed.vary_accept);
            parsed.response_type.is_text()
        }
        assert!(text("text/plain").await);
        assert!(text("text/markdown").await);
        assert!(text("Text/Markdown;charset=utf-8").await);
        assert!(text("text/markdown, text/html;q=0.5").await);
        assert!(text("text/markdown, text/html;q=0.9, */*;q=0.8").await);
        assert!(!text("text/html, text/markdown;q=0.9").await);
        assert!(!text("text/html;q=0.9, text/markdown;q=0.9").await);
        assert!(!text("text/plain;q=0, text/html;q=0.1").await);
        assert!(text("text/markdown, text/plain;q=0.9, */*;q=0.1").await);
        assert!(!text("text/html,application/xhtml+xml,*/*;q=0.8").await);
        assert!(!text("text/plain, text/html").await);
        assert!(!text("*/*").await);

        let parsed = parse(&request_accept("/text", "text/html")).await.unwrap();
        assert!(parsed.response_type.is_text());
        assert!(!parsed.vary_accept);
    }

    fn request(uri: &str) -> Request<Bytes> {
        Request::builder()
            .uri(uri)
            .body(Bytes::new())
            .expect("valid request")
    }

    #[tokio::test]
    async fn parse_paginated_address() {
        let address = "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq";
        let parsed = parse(&request(&format!("/a/{address}/text")))
            .await
            .unwrap();
        match parsed.resource {
            Resource::Address(ref a, None, 0) => {
                assert_eq!(a.clone().assume_checked().to_string(), address)
            }
            ref resource => panic!("unexpected resource: {resource:?}"),
        }
        assert!(parsed.response_type.is_text());

        let parsed = parse(&request(&format!("/a/{address}/3?amount=1")))
            .await
            .unwrap();
        match parsed.resource {
            Resource::Address(_, Some(ref query), 3) => assert_eq!(query, "amount=1"),
            ref resource => panic!("unexpected resource: {resource:?}"),
        }

        assert!(parse(&request(&format!("/a/{address}/x"))).await.is_err());
    }

    #[tokio::test]
    async fn parse_tx_ignores_query_string() {
        let txid =
            Txid::from_str("e5f65dd2b995966c74a5beec6a09934864fbdb64e7fb6b3b487e6bb9af2b236c")
                .unwrap();

        let parsed = parse(&request(
            "/t/e5f65dd2b995966c74a5beec6a09934864fbdb64e7fb6b3b487e6bb9af2b236c?mode=details",
        ))
        .await
        .unwrap();

        match parsed.resource {
            Resource::Tx(parsed_txid, page) => {
                assert_eq!(parsed_txid, txid);
                assert_eq!(page, 0);
            }
            resource => panic!("unexpected resource: {resource:?}"),
        }
    }

    #[tokio::test]
    async fn parse_paginated_tx_ignores_query_string() {
        let txid =
            Txid::from_str("e5f65dd2b995966c74a5beec6a09934864fbdb64e7fb6b3b487e6bb9af2b236c")
                .unwrap();

        let parsed = parse(&request(
            "/t/e5f65dd2b995966c74a5beec6a09934864fbdb64e7fb6b3b487e6bb9af2b236c/2?mode=details",
        ))
        .await
        .unwrap();

        match parsed.resource {
            Resource::Tx(parsed_txid, page) => {
                assert_eq!(parsed_txid, txid);
                assert_eq!(page, 2);
            }
            resource => panic!("unexpected resource: {resource:?}"),
        }
    }

    #[tokio::test]
    async fn parse_tx_redirect_ignores_query_string() {
        let txid =
            Txid::from_str("e5f65dd2b995966c74a5beec6a09934864fbdb64e7fb6b3b487e6bb9af2b236c")
                .unwrap();

        let parsed = parse(&request(
            "/tx/e5f65dd2b995966c74a5beec6a09934864fbdb64e7fb6b3b487e6bb9af2b236c?mode=details",
        ))
        .await
        .unwrap();

        match parsed.resource {
            Resource::TxToT(parsed_txid) => assert_eq!(parsed_txid, txid),
            resource => panic!("unexpected resource: {resource:?}"),
        }
    }
}
