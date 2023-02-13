mod address;
mod blockhash;
mod mempoolfees;
mod outpoint;
mod script;
mod txid;
mod witness;

pub trait Html {
    fn html(&self) -> maud::Markup;
}
