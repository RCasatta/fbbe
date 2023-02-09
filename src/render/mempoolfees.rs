use super::Html;
use crate::state::MempoolFees;
use maud::{html, Render};

impl Render for MempoolFees {
    fn render(&self) -> maud::Markup {
        html! {

            @if let Some(highest) = self.highest  {
                tr {
                    th { em data-tooltip="Transaction with the highest fee in the mempool" { "Highest" } }
                    td class="right" { (highest.html()) }
                }
            }

            @if let Some(middle_in_block) = self.middle_in_block  {
                tr {
                    th { em data-tooltip="A transaction in the middle of a block template created with current mempool" { "Middle in block" } }
                    td class="right" { (middle_in_block.html()) }
                }
            }

            @if let Some(last_in_block) = self.last_in_block  {
                tr {
                    th { em data-tooltip="The last transaction (lowest fee) of a block template created with current mempool" { "Last in block" } }
                    td class="right" { (last_in_block.html()) }
                }
            }


        }
    }
}
