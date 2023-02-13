use bitcoin::{Amount, Denomination};
use maud::{html, Render};

pub struct AmountRow<'a> {
    title: &'a str,
    amount: String,
}

impl<'a> AmountRow<'a> {
    pub fn new_with_sat(title: &'a str, amount: u64) -> Self {
        Self {
            title,
            amount: format!(
                "{:.8}",
                Amount::from_sat(amount).to_float_in(Denomination::Bitcoin)
            ),
        }
    }
    pub fn new_with_btc(title: &'a str, amount: f64) -> Self {
        Self {
            title,
            amount: format!("{:.8}", amount),
        }
    }
}

impl<'a> Render for AmountRow<'a> {
    fn render(&self) -> maud::Markup {
        html! {
            tr {
                th { (self.title) }
                td class="number" { (self.amount) }
            }
        }
    }
}
