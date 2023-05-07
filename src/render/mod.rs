mod address;
mod amountrow;
mod blockhash;
mod humanbytes;
mod mempool;
mod outpoint;
mod script;
mod sizerow;
mod txid;
mod witness;

pub use amountrow::AmountRow;
pub use mempool::MempoolSection;
pub use sizerow::SizeRow;

pub trait Html {
    fn html(&self) -> maud::Markup;
}
