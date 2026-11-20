use crate::route::ResponseType;
use bitcoin::{consensus::encode, Address, BlockHash, Network, Txid};
use hyper::StatusCode;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Serde(#[from] serde_json::Error),

    #[error(transparent)]
    Hyper(#[from] hyper::Error),

    #[error(transparent)]
    HyperUtil(#[from] hyper_util::client::legacy::Error),

    #[error(transparent)]
    HyperHttp(#[from] hyper::http::Error),

    #[error(transparent)]
    Uri(#[from] hyper::http::uri::InvalidUri),

    #[error(transparent)]
    Hex(#[from] bitcoin::hashes::hex::HexToBytesError),

    #[error(transparent)]
    HexArray(#[from] bitcoin::hashes::hex::HexToArrayError),

    #[error(transparent)]
    Hex2(#[from] hex::FromHexError),

    #[error(transparent)]
    Html2Text(#[from] html2text::Error),

    #[error(transparent)]
    ParseInt(#[from] std::num::ParseIntError),

    #[error(transparent)]
    ParseOutPointError(#[from] bitcoin::transaction::ParseOutPointError),

    #[error(transparent)]
    Bitcoin(#[from] encode::Error),

    #[error(transparent)]
    ParseNetworkError(#[from] bitcoin::network::ParseNetworkError),

    #[error(transparent)]
    Join(#[from] tokio::task::JoinError),

    #[error(transparent)]
    BitcoinAddress(#[from] bitcoin::address::ParseError),

    #[error(transparent)]
    Bmp(#[from] qr_code::bmp_monochrome::BmpError),

    #[error(transparent)]
    Qr(#[from] qr_code::types::QrError),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Rocksdb(#[from] rocksdb::Error),

    #[error(transparent)]
    Prometheus(#[from] prometheus::Error),

    #[error(transparent)]
    Zmq(#[from] async_zmq::Error),

    #[error(transparent)]
    ZmqSubscribe(#[from] async_zmq::SubscribeError),

    #[error(transparent)]
    ExtractTx(#[from] bitcoin::psbt::ExtractTxError),

    #[error("Bitcoin core RPC chaininfo failed status_code:{0}")]
    RpcChainInfo(StatusCode),

    #[error("Bitcoin core RPC tx failed. txid:{1} status_code:{0}")]
    RpcTx(StatusCode, Txid),

    #[error("Bitcoin core RPC tx json failed. txid:{1} status_code:{0}")]
    RpcTxJson(StatusCode, Txid),

    #[error("Bitcoin core RPC txout failed. txid:{1} vout:{2} status_code:{0}")]
    RpcTxOut(StatusCode, Txid, u32),

    #[error("Bitcoin core RPC block json failed. block_hash:{1} status_code:{0}")]
    RpcBlockJson(StatusCode, BlockHash),

    #[error("Bitcoin core RPC block hash by height json failed. height:{1} status_code:{0}")]
    RpcBlockHashByHeightJson(StatusCode, usize),

    #[error("Bitcoin core RPC block header json failed for block. block_hash:{1} status_code:{0}")]
    RpcBlockHeaderJson(StatusCode, BlockHash),

    #[error("Bitcoin core RPC block raw failed for block. block_hash:{1} status_code:{0}")]
    RpcBlockRaw(StatusCode, BlockHash),

    #[error("Bitcoin core RPC headers failed. start:{1} count:{2} status_code:{0}")]
    RpcBlockHeaders(StatusCode, BlockHash, u32),

    #[error("Bitcoin core RPC mempool info failed. status_code:{0}")]
    RpcMempoolInfo(StatusCode),

    #[error("Bitcoin core RPC mempool content failed. status_code:{0}")]
    RpcMempoolContent(StatusCode),

    #[error("Invalid page number")]
    InvalidPageNumber,

    #[error("Bad request")]
    BadRequest,

    #[error("Bad request: {0}")]
    InvalidInput(String),

    #[error("Page not found")]
    NotFound,

    #[error("Header not found {0}")]
    HeaderNotFound(BlockHash),

    #[error("Genesis tx doesn't really exist, it's unspendable")]
    GenesisTx,

    #[error("Content type {0:?} is not supported for endpoint {1}")]
    ContentTypeUnsupported(ResponseType, String),

    #[error("bitcoind is started without the rest flag (`rest=1` in `bitcoin.conf` or `--rest`)")]
    RestFlag,

    #[error("bitcoind and fbbe doesn't have the same network. fbbe:{fbbe} bitcoind:{bitcoind}")]
    WrongNetwork { fbbe: Network, bitcoind: Network },

    #[error("address and fbbe doesn't have the same network. fbbe:{fbbe} address:{address}")]
    AddressWrongNetwork { fbbe: Network, address: Address },

    #[error("Network '{0}' not parsed, valid values are: bitcoin, mainnet, main | testnet, test | signet | regtest")]
    NetworkParseError(String),

    #[error("Block at height {0} not found")]
    HeightNotFound(u32),
}

impl Error {
    /// Wraps errors happening while parsing the user request, they are caused by invalid input
    pub fn into_input_error(self) -> Self {
        match self {
            Error::BadRequest | Error::NotFound | Error::InvalidInput(_) => self,
            e => Error::InvalidInput(e.to_string()),
        }
    }

    pub fn status_code(&self) -> StatusCode {
        match self {
            Error::BadRequest
            | Error::InvalidInput(_)
            | Error::InvalidPageNumber
            | Error::ContentTypeUnsupported(_, _)
            | Error::BitcoinAddress(_)
            | Error::AddressWrongNetwork { .. } => StatusCode::BAD_REQUEST,

            Error::NotFound | Error::HeightNotFound(_) | Error::HeaderNotFound(_) => {
                StatusCode::NOT_FOUND
            }

            Error::RpcTx(s, _)
            | Error::RpcTxJson(s, _)
            | Error::RpcTxOut(s, _, _)
            | Error::RpcBlockJson(s, _)
            | Error::RpcBlockRaw(s, _)
            | Error::RpcBlockHeaderJson(s, _)
            | Error::RpcBlockHashByHeightJson(s, _)
                if *s == StatusCode::NOT_FOUND =>
            {
                StatusCode::NOT_FOUND
            }

            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// Message shown to the user, without node internals when the resource is simply missing
    pub fn public_message(&self) -> String {
        if self.status_code() == StatusCode::NOT_FOUND {
            match self {
                Error::RpcTx(_, txid) | Error::RpcTxJson(_, txid) => {
                    return format!("Transaction {txid} not found")
                }
                Error::RpcTxOut(_, txid, vout) => return format!("Output {txid}:{vout} not found"),
                Error::RpcBlockJson(_, hash)
                | Error::RpcBlockRaw(_, hash)
                | Error::RpcBlockHeaderJson(_, hash)
                | Error::HeaderNotFound(hash) => return format!("Block {hash} not found"),
                Error::RpcBlockHashByHeightJson(_, height) => {
                    return format!("Block at height {height} not found")
                }
                _ => (),
            }
        }
        self.to_string()
    }
}
