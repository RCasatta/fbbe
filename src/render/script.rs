use super::Html;
use maud::{html, Render};

pub(crate) struct Script<'a>(&'a bitcoin::Script);

impl<'a> Render for Script<'a> {
    fn render(&self) -> maud::Markup {
        let asm = self.0.asm();
        let pieces = asm.split(" ");
        html! {
            code {
                small {
                    @for piece in pieces {
                        @if piece.starts_with("OP_") {
                            b { (piece) }
                        } @else {
                            (piece)
                        }
                        " "
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
