use std::fmt::Display;

use super::Html;
use crate::{network, NetworkExt};
use maud::{html, Render};

pub(crate) struct Address<'a>(&'a bitcoin::Address);
struct Link<'a>(&'a bitcoin::Address);
impl<'a> Display for Link<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}a/{}", network().as_url_path(), self.0)
    }
}

impl<'a> Render for Address<'a> {
    fn render(&self) -> maud::Markup {
        let link = Link(self.0);

        html! {
            a href=(link) { code { span class="address" { (self.0) } } }
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
