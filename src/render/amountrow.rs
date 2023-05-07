use std::fmt::Display;

use bitcoin::Denomination;
use maud::{html, Render};

pub struct AmountRow<'a> {
    title: &'a str,
    amount: Amount,
}

struct Amount(bitcoin::Amount);
impl Display for Amount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:.8}", self.0.to_float_in(Denomination::Bitcoin))
    }
}

impl<'a> AmountRow<'a> {
    pub fn new_with_sat(title: &'a str, amount: u64) -> Self {
        Self {
            title,
            amount: Amount(bitcoin::Amount::from_sat(amount)),
        }
    }
    pub fn new_with_btc(title: &'a str, amount: f64) -> Self {
        Self {
            title,
            amount: Amount(bitcoin::Amount::from_float_in(amount, Denomination::Bitcoin).unwrap()),
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
