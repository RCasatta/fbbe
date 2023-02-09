mod blockhash;
mod mempoolfees;
mod outpoint;
mod txid;

pub trait Html {
    fn html(&self) -> maud::Markup;
}
