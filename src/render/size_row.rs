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
        html! {
            tr {
                th { (self.title) }
                td class="right" { (size_str(self.size)) }
            }
        }
    }
}

/// Exact bytes, with the rounded value in front when it is shorter, like `2.0 KB (2013 B)`
fn size_str(size: u64) -> String {
    if size < 1000 {
        format!("{size} B")
    } else {
        format!("{} ({size} B)", HumanBytes::new(size as f64))
    }
}

#[cfg(test)]
mod tests {
    use super::size_str;

    #[test]
    fn exact_size() {
        assert_eq!(size_str(0), "0 B");
        assert_eq!(size_str(227), "227 B");
        assert_eq!(size_str(2013), "2.0 KB (2013 B)");
        assert_eq!(size_str(1_534_233), "1.5 MB (1534233 B)");
    }
}
