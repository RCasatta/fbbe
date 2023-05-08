mod address;
mod amount_row;
mod block_hash;
mod human_bytes;
mod mempool;
mod outpoint;
mod plural;
mod script;
mod size_row;
mod txid;
mod witness;

pub use amount_row::AmountRow;
pub use mempool::MempoolSection;
pub use plural::Plural;
pub use size_row::SizeRow;

pub trait Html {
    fn html(&self) -> maud::Markup;
}
