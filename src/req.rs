use std::str::FromStr;

use crate::error::Error;
use bitcoin::{Address, BlockHash, Txid};
use bitcoin_hashes::{hex::FromHex, sha256d};
use hyper::{Body, Method, Request};

#[derive(Debug, Clone)]
pub enum ParsedRequest {
    Home,
    Favicon,
    Css,
    CustomCss,
    Contact,
    SearchHeight(u32),
    SearchBlock(BlockHash),
    SearchTx(Txid),
    SearchAddress(Address),
    Tx(Txid),
    Block(BlockHash, usize),
    TxOut(Txid, u32),
    Head,
    Robots,
    BlockToB(BlockHash),
    TxToT(Txid),
    Address(Address),
    AddressToA(Address),
}

pub async fn parse(req: &Request<Body>) -> Result<ParsedRequest, Error> {
    let mut path = req.uri().path().split('/').skip(1);
    let query = req.uri().query();
    let is_head = req.method() == Method::HEAD;
    let method = if is_head { &Method::GET } else { req.method() };

    let parsed = match (method, query, path.next(), path.next(), path.next()) {
        (&Method::GET, None, Some(""), None, None) => ParsedRequest::Home,
        (&Method::GET, Some(query), None | Some(""), None, None) => {
            if query.contains('&') {
                return Err(Error::BadRequest);
            }
            let mut iter = query.split('=');
            if iter.next() != Some("s") {
                return Err(Error::BadRequest);
            }
            match (iter.next(), iter.next()) {
                (Some(val), None) => match val.parse::<u32>() {
                    Ok(height) => ParsedRequest::SearchHeight(height),
                    Err(_) => match sha256d::Hash::from_hex(val) {
                        Ok(val) => {
                            if val.ends_with(&[0u8; 4]) {
                                ParsedRequest::SearchBlock(val.into())
                            } else {
                                ParsedRequest::SearchTx(val.into())
                            }
                        }
                        Err(_) => match Address::from_str(val) {
                            Ok(address) => ParsedRequest::SearchAddress(address),
                            Err(_) => return Err(Error::BadRequest),
                        },
                    },
                },
                _ => return Err(Error::BadRequest),
            }
        }

        (&Method::GET, None, Some("favicon.ico"), None, None) => ParsedRequest::Favicon,
        (&Method::GET, None, Some("robots.txt"), None, None) => ParsedRequest::Robots,
        (&Method::GET, None, Some("css"), Some("pico.min.css"), None) => ParsedRequest::Css,
        (&Method::GET, None, Some("css"), Some("custom.css"), None) => ParsedRequest::CustomCss,
        (&Method::GET, None, Some("contact"), None, None) => ParsedRequest::Contact,

        (&Method::GET, None, Some("t"), Some(txid), None) => {
            let txid = Txid::from_hex(txid)?;
            ParsedRequest::Tx(txid)
        }
        (&Method::GET, None, Some("o"), Some(txid), Some(vout)) => {
            let txid = Txid::from_hex(txid)?;
            let vout: u32 = vout.parse()?;
            ParsedRequest::TxOut(txid, vout)
        }
        (&Method::GET, None, Some("h"), Some(height), None) => {
            let height: u32 = height.parse()?;
            ParsedRequest::SearchHeight(height)
        }
        (&Method::GET, None, Some("b"), Some(block_hash), page) => {
            let block_hash = BlockHash::from_hex(block_hash)?;
            let page = match page {
                Some(page) => page.parse::<usize>()?,
                None => 0,
            };
            ParsedRequest::Block(block_hash, page)
        }
        (&Method::GET, None, Some("a"), Some(address), None) => {
            let address = Address::from_str(address)?;
            ParsedRequest::Address(address)
        }
        (&Method::GET, None, Some("block"), Some(block_hash), None) => {
            let block_hash = BlockHash::from_hex(block_hash)?;
            ParsedRequest::BlockToB(block_hash)
        }
        (&Method::GET, None, Some("tx"), Some(txid), None) => {
            let txid = Txid::from_hex(txid)?;
            ParsedRequest::TxToT(txid)
        }
        (&Method::GET, None, Some("address"), Some(address), None) => {
            let address = Address::from_str(address)?;
            ParsedRequest::AddressToA(address)
        }
        _ => return Err(Error::NotFound),
    };

    if is_head {
        Ok(ParsedRequest::Head)
    } else {
        Ok(parsed)
    }
}
