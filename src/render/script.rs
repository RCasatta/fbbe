use super::Html;
use maud::{html, Render};

pub(crate) struct Script<'a>(&'a bitcoin::Script);

impl<'a> Render for Script<'a> {
    fn render(&self) -> maud::Markup {
        let asm = if self.0.is_empty() {
            "<empty>".to_owned()
        } else {
            self.0.to_asm_string()
        };
        let pieces = asm.split(' ');
        html! {
            code {
                @for (i, piece) in pieces.enumerate() {
                    @if i != 0 {
                        " "
                    }
                    @if piece.starts_with("OP_") {
                        span class="script" { (piece) }
                    } @else {
                        (piece)
                    }

                }

            }
        }
    }
}

impl Html for bitcoin::Script {
    fn html(&self) -> maud::Markup {
        Script(self).render()
    }
}

impl<'a> From<&'a bitcoin::Script> for Script<'a> {
    fn from(a: &'a bitcoin::Script) -> Self {
        Script(a)
    }
}
