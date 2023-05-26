use crate::route::ResponseType;
use bitcoin::{consensus::encode, BlockHash, Network, Txid};
use hyper::StatusCode;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Serde(#[from] serde_json::Error),

    #[error(transparent)]
    Hyper(#[from] hyper::Error),

    #[error(transparent)]
    HyperHttp(#[from] hyper::http::Error),

    #[error(transparent)]
    Uri(#[from] hyper::http::uri::InvalidUri),

    #[error(transparent)]
    Hex(#[from] bitcoin::hashes::hex::Error),

    #[error(transparent)]
    ParseInt(#[from] std::num::ParseIntError),

    #[error(transparent)]
    Bitcoin(#[from] encode::Error),

    #[error(transparent)]
    ParseNetworkError(#[from] bitcoin::network::constants::ParseNetworkError),

    #[error(transparent)]
    Join(#[from] tokio::task::JoinError),

    #[error(transparent)]
    BitcoinAddress(#[from] bitcoin::address::Error),

    #[error(transparent)]
    Bmp(#[from] qr_code::bmp_monochrome::BmpError),

    #[error(transparent)]
    Qr(#[from] qr_code::types::QrError),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error("Bitcoin core RPC chaininfo failed status_code:{0}")]
    RpcChainInfo(StatusCode),

    #[error("Bitcoin core RPC tx failed. txid:{1} status_code:{0}")]
    RpcTx(StatusCode, Txid),

    #[error("Bitcoin core RPC tx json failed. txid:{1} status_code:{0}")]
    RpcTxJson(StatusCode, Txid),

    #[error("Bitcoin core RPC txout failed. txid:{1} vout:{2} status_code:{0}")]
    RpcTxOut(StatusCode, Txid, u32),

    #[error("Bitcoin core RPC block json failed. block_hash:{0} status_code:{0}")]
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
    AddressWrongNetwork { fbbe: Network, address: Network },

    #[error("Network '{0}' not parsed, valid values are: bitcoin, mainnet, main | testnet, test | signet | regtest")]
    NetworkParseError(String),
}

impl From<Error> for StatusCode {
    fn from(e: Error) -> Self {
        match e {
            Error::BadRequest => StatusCode::BAD_REQUEST,
            Error::NotFound => StatusCode::NOT_FOUND,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}
