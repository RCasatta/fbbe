mod address;
mod blockhash;
mod mempool;
mod outpoint;
mod script;
mod txid;
mod witness;

pub use mempool::MempoolSection;

pub trait Html {
    fn html(&self) -> maud::Markup;
}
