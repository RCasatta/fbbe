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
use hyper::{Body, Method, Request};

#[derive(Debug, Clone)]
pub struct ParsedRequest {
    pub resource: Resource,
    pub response_type: ResponseType,
}

#[derive(Debug, Clone)]
pub enum Resource {
    Home,
    Favicon,
    Css,
    Contact,
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
    Address(Address<NetworkUnchecked>, Option<String>),
    AddressToA(Address),
    FullTx(Transaction),
    Metrics,
    Sitemap,
}

pub async fn parse(req: &Request<Body>) -> Result<ParsedRequest, Error> {
    let mut path: Vec<_> = req.uri().path().split('/').skip(1).take(5).collect();
    log::debug!("{:?}", path);

    if path.get(4).is_some() {
        return Err(Error::BadRequest);
    }
    let response_type = match path.last() {
        Some(&"text") => ResponseType::Text(parse_cols(req)),
        Some(&"bin") => ResponseType::Bytes,
        _ => ResponseType::Html,
    };
    log::debug!("{:?}", response_type);
    if let ResponseType::Text(_) | ResponseType::Bytes = response_type {
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
                                            .map_err(|_| Error::BadRequest)?;
                                        let psbt = Psbt::from_str(val.as_ref())
                                            .map_err(|_| Error::BadRequest)?;
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
        (&Method::GET, None, Some(&"contact"), None, None) => Resource::Contact,
        (&Method::GET, None, Some(&"metrics"), None, None) => Resource::Metrics,
        (&Method::GET, None, Some(&"sitemap.xml"), None, None) => Resource::Sitemap,

        (&Method::GET, None, Some(&"t"), Some(txid), page) => {
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
        (&Method::GET, query, Some(&"a"), Some(address), None) => {
            let address = Address::from_str(address)?;
            Resource::Address(address, query.map(ToString::to_string))
        }
        (&Method::GET, None, Some(&"block"), Some(block_hash), None) => {
            let block_hash = BlockHash::from_str(block_hash)?;
            Resource::BlockToB(block_hash)
        }
        (&Method::GET, None, Some(&"tx"), Some(txid), None) => {
            let txid = Txid::from_str(txid)?;
            Resource::TxToT(txid)
        }
        (&Method::GET, None, Some(&"txhex"), Some(hex), None) => {
            let bytes = hex::decode(hex)?;
            let tx: Transaction = deserialize(&bytes)?;
            Resource::FullTx(tx)
        }
        (&Method::GET, None, Some(&"address"), Some(address), None) => {
            let address = Address::from_str(address)?;
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
    })
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
            Resource::Address(address, query) => {
                let address = address.clone().assume_checked(); // TODO clone is a performance penalty here
                write!(f, "{base}a/{address}/text")?;
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
            Home | Tx(_, _) | Block(_, _) | Address(_, _) => Some(TextLink(self)),
            _ => None,
        }
    }
}

fn parse_cols(req: &Request<Body>) -> u16 {
    req.headers()
        .get("columns")
        .and_then(|c| c.to_str().ok())
        .and_then(|e| e.parse::<u16>().ok())
        .unwrap_or(80)
}
