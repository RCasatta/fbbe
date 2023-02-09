mod address;
mod blockhash;
mod mempoolfees;
mod outpoint;
mod script;
mod txid;

pub trait Html {
    fn html(&self) -> maud::Markup;
}
