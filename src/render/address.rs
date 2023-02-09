use super::Html;
use maud::{html, Render};

pub(crate) struct Address<'a>(&'a bitcoin::Address);

impl<'a> Render for Address<'a> {
    fn render(&self) -> maud::Markup {
        html! {
             code { (self.0.to_string()) }
        }
    }
}

impl Html for bitcoin::Address {
    fn html(&self) -> maud::Markup {
        Address(self).render()
    }
}

impl<'a> From<&'a bitcoin::Address> for Address<'a> {
    fn from(a: &'a bitcoin::Address) -> Self {
        Address(a)
    }
}
