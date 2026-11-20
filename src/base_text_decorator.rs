use html2text::render::{RichAnnotation, TaggedLine, TextDecorator};

use crate::globals::dns_host;
use bitcoin::Network;

pub(crate) struct BaseTextDecorator;

impl BaseTextDecorator {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {}
    }
}

impl TextDecorator for BaseTextDecorator {
    type Annotation = RichAnnotation;

    fn decorate_link_start(&mut self, url: &str) -> (String, Self::Annotation) {
        ("".to_string(), RichAnnotation::Link(url.to_string()))
    }

    fn decorate_link_end(&mut self) -> String {
        "".to_string()
    }

    fn decorate_em_start(&self) -> (String, Self::Annotation) {
        ("".to_string(), RichAnnotation::Emphasis)
    }

    fn decorate_em_end(&self) -> String {
        "".to_string()
    }

    fn decorate_strong_start(&self) -> (String, Self::Annotation) {
        ("*".to_string(), RichAnnotation::Strong)
    }

    fn decorate_strong_end(&self) -> String {
        "*".to_string()
    }

    fn decorate_strikeout_start(&self) -> (String, Self::Annotation) {
        ("".to_string(), RichAnnotation::Strikeout)
    }

    fn decorate_strikeout_end(&self) -> String {
        "".to_string()
    }

    fn decorate_code_start(&self) -> (String, Self::Annotation) {
        (String::new(), RichAnnotation::Code)
    }

    fn decorate_code_end(&self) -> String {
        String::new()
    }

    fn decorate_preformat_first(&self) -> Self::Annotation {
        RichAnnotation::Preformat(false)
    }

    fn decorate_preformat_cont(&self) -> Self::Annotation {
        RichAnnotation::Preformat(true)
    }

    fn decorate_image(&mut self, src: &str, title: &str) -> (String, Self::Annotation) {
        (title.to_string(), RichAnnotation::Image(src.to_string()))
    }

    fn header_prefix(&self, level: usize) -> String {
        "#".repeat(level) + " "
    }

    fn quote_prefix(&self) -> String {
        "> ".to_string()
    }

    fn unordered_item_prefix(&self) -> String {
        "* ".to_string()
    }

    fn ordered_item_prefix(&self, i: i64) -> String {
        format!("{}. ", i)
    }

    fn finalise(&mut self, links: Vec<String>) -> Vec<TaggedLine<Self::Annotation>> {
        links
            .into_iter()
            .enumerate()
            .map(|(i, link)| {
                let line = format!("[{}]: {}", i + 1, footnote_url(&link, dns_host()));
                TaggedLine::from_string(line, &Default::default())
            })
            .collect()
    }

    fn make_subblock_decorator(&self) -> Self {
        Self::new()
    }
}

/// Internal links to pages having a text version are converted to point to it, so that following
/// links from a text page remains in text, if `dns_host` is given links are made absolute.
pub(crate) fn footnote_url(link: &str, dns_host: Option<&str>) -> String {
    if !link.starts_with('/') {
        return link.to_string();
    }
    let link = link.split('#').next().unwrap_or_default();
    let (path, query) = match link.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (link, None),
    };
    let mut segments: Vec<_> = path.split('/').filter(|s| !s.is_empty()).collect();
    let resource = match segments.first() {
        Some(net) if net.parse::<Network>().is_ok() => &segments[1..],
        _ => &segments[..],
    };
    let has_text = match resource {
        [] => true, // home
        [kind, _id] => matches!(*kind, "t" | "b" | "a"),
        [kind, _id, page] => matches!(*kind, "t" | "b") && page.parse::<usize>().is_ok(),
        _ => false,
    };
    if has_text {
        segments.push("text");
    }
    let mut result = String::new();
    if let Some(dns_host) = dns_host {
        result.push_str("https://");
        result.push_str(dns_host);
    }
    for segment in segments {
        result.push('/');
        result.push_str(segment);
    }
    if let Some(query) = query {
        result.push('?');
        result.push_str(query);
    }
    result
}
