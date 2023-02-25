use super::Html;
use crate::{network, NetworkExt};
use maud::{html, Render};

pub(crate) struct Address<'a>(&'a bitcoin::Address);

impl<'a> Render for Address<'a> {
    fn render(&self) -> maud::Markup {
        let address_string = self.0.to_string();
        let network_url_path = network().as_url_path();
        let link = format!("{network_url_path}a/{address_string}");

        html! {
            a href=(link) {  code { (address_string) } }
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
