mod address;
mod address_seen;
mod amount_row;
mod block_hash;
mod human_bytes;
mod mempool;
mod outpoint;
mod plural;
mod script;
mod size_row;
mod spending;
mod txid;
mod witness;

pub use amount_row::AmountRow;
pub use block_hash::BlockHash;
pub use mempool::MempoolSection;
pub use outpoint::OutPoint;
pub use plural::Plural;
pub use size_row::SizeRow;
pub use txid::Txid;

pub trait Html {
    fn html(&self) -> maud::Markup;
}
