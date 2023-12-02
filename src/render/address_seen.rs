use maud::{html, Render};

use crate::{render::Html, threads::index_addresses::AddressSeen};

impl Render for AddressSeen {
    fn render(&self) -> maud::Markup {
        html! {

            div { "Funding @ " (self.funding.height_time.date_time_utc())}
            p { (self.funding.out_point.html()) }

            @if let Some(spending) = self.spending.as_ref() {
                div { "Spending @ " (spending.height_time.date_time_utc())}
                    p { (spending) }
                }
        }
    }
}
