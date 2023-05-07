mod address;
mod amountrow;
mod blockhash;
mod human_bytes;
mod mempool;
mod outpoint;
mod script;
mod size_row;
mod txid;
mod witness;

pub use amountrow::AmountRow;
pub use mempool::MempoolSection;
pub use size_row::SizeRow;

pub trait Html {
    fn html(&self) -> maud::Markup;
}
