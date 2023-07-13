use html2text::render::text_renderer::{RichAnnotation, TaggedLine, TextDecorator};

pub(crate) struct BaseTextDecorator;

impl BaseTextDecorator {
    #[cfg_attr(feature = "clippy", allow(new_without_default_derive))]
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

    fn decorate_em_start(&mut self) -> (String, Self::Annotation) {
        ("".to_string(), RichAnnotation::Emphasis)
    }

    fn decorate_em_end(&mut self) -> String {
        "".to_string()
    }

    fn decorate_strong_start(&mut self) -> (String, Self::Annotation) {
        ("*".to_string(), RichAnnotation::Strong)
    }

    fn decorate_strong_end(&mut self) -> String {
        "*".to_string()
    }

    fn decorate_strikeout_start(&mut self) -> (String, Self::Annotation) {
        ("".to_string(), RichAnnotation::Strikeout)
    }

    fn decorate_strikeout_end(&mut self) -> String {
        "".to_string()
    }

    fn decorate_code_start(&mut self) -> (String, Self::Annotation) {
        (String::new(), RichAnnotation::Code)
    }

    fn decorate_code_end(&mut self) -> String {
        String::new()
    }

    fn decorate_preformat_first(&mut self) -> Self::Annotation {
        RichAnnotation::Preformat(false)
    }

    fn decorate_preformat_cont(&mut self) -> Self::Annotation {
        RichAnnotation::Preformat(true)
    }

    fn decorate_image(&mut self, src: &str, title: &str) -> (String, Self::Annotation) {
        (title.to_string(), RichAnnotation::Image(src.to_string()))
    }

    fn header_prefix(&mut self, level: usize) -> String {
        "#".repeat(level) + " "
    }

    fn quote_prefix(&mut self) -> String {
        "> ".to_string()
    }

    fn unordered_item_prefix(&mut self) -> String {
        "* ".to_string()
    }

    fn ordered_item_prefix(&mut self, i: i64) -> String {
        format!("{}. ", i)
    }

    fn finalise(&mut self, _links: Vec<String>) -> Vec<TaggedLine<Self::Annotation>> {
        Vec::new()
    }

    fn make_subblock_decorator(&self) -> Self {
        Self::new()
    }
}
