use crate::route::ResponseType;
use bitcoin::{consensus::encode, BlockHash, Txid};
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
    Hex(#[from] bitcoin_hashes::hex::Error),

    #[error(transparent)]
    ParseInt(#[from] std::num::ParseIntError),

    #[error(transparent)]
    Bitcoin(#[from] encode::Error),

    #[error(transparent)]
    Join(#[from] tokio::task::JoinError),

    #[error("Bitcoin core RPC chaininfo failed")]
    RpcChainInfo,

    #[error("Bitcoin core RPC tx failed txid:{0}")]
    RpcTx(Txid),

    #[error("Bitcoin core RPC tx json failed txid:{0}")]
    RpcTxJson(Txid),

    #[error("Bitcoin core RPC txout failed txid:{0} vout:{1}")]
    RpcTxOut(Txid, u32),

    #[error("Bitcoin core RPC block json failed {0}")]
    RpcBlockJson(BlockHash),

    #[error("Bitcoin core RPC block hash by height ({0}) json failed")]
    RpcBlockHashByHeightJson(usize),

    #[error("Bitcoin core RPC block header json failed for block hash:{0}")]
    RpcBlockHeaderJson(BlockHash),

    #[error("Bitcoin core RPC block raw failed for block {0}")]
    RpcBlockRaw(BlockHash),

    #[error("Bitcoin core RPC headers failed start:{0} count:{1}")]
    RpcBlockHeaders(BlockHash, u32),

    #[error("Bitcoin core RPC mempool info failed")]
    RpcMempoolInfo,

    #[error("Bitcoin core RPC mempool content failed")]
    RpcMempoolContent,

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
