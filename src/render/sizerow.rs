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

impl<'a> Render for SizeRow<'a> {
    fn render(&self) -> maud::Markup {
        html! {
            tr {
                th { (self.title) }
                td class="right" { (human_bytes::human_bytes(self.size as f64)) }
            }
        }
    }
}
