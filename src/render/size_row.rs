use crate::render::human_bytes::HumanBytes;
use maud::{html, Render};

pub struct SizeRow<'a> {
    title: &'a str,
    size: u64,
}

impl<'a> SizeRow<'a> {
    pub fn new(title: &'a str, size: u64) -> Self {
        Self { title, size }
    }
}

impl Render for SizeRow<'_> {
    fn render(&self) -> maud::Markup {
        let hb = HumanBytes::new(self.size as f64);
        html! {
            tr {
                th { (self.title) }
                td class="right" { (hb) }
            }
        }
    }
}
