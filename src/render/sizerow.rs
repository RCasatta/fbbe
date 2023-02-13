use maud::{html, Render};
use thousands::{digits, Separable, SeparatorPolicy};

pub struct SizeRow<'a> {
    title: &'a str,
    size: usize,
}

impl<'a> SizeRow<'a> {
    pub fn new(title: &'a str, size: usize) -> Self {
        Self { title, size }
    }
}

impl<'a> Render for SizeRow<'a> {
    fn render(&self) -> maud::Markup {
        html! {
            tr {
                th { (self.title) }
                td class="right" { (self.size.separate_by_policy(SEPARATOR_POLICY)) }
            }
        }
    }
}

const SEPARATOR_POLICY: SeparatorPolicy = SeparatorPolicy {
    separator: " ", // NARROW NO-BREAK SPACE' (U+202F)
    groups: &[3],
    digits: digits::ASCII_DECIMAL,
};
