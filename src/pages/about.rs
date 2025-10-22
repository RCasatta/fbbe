use super::html_page;
use crate::{error::Error, req::ParsedRequest};
use maud::{html, Markup};

pub fn page(parsed: &ParsedRequest) -> Result<Markup, Error> {
    let content = html! {
        section {
            h1 { "About" }

            p { "A Fast Bitcoin Block Explorer with the following features:" }
            ul {
                li { "simple: requires only a " a href="https://github.com/bitcoin/bitcoin" { "bitcoin core" } " backend with rest and txindex enabled." }
                li { "bitcoin-only: mainnet, signet and regtest are supported." }
                li { "cache-friendly: pages intentionally avoid fields like 'confirmations' so that the page is cache-able forever." }
                li { "terminal-friendly: the simple standard html is easily rendered in text to be shown in a terminal, add `/text` to any page like the " a href="https://fbbe.info/text" { "home" } " page." }
                li { "low-bandwidth: no images, no blocking javascript, no external fonts, small html, a single external stylesheets which is preloaded. After the first, a single network request load any page." }
                li { "privacy-friendly: we use a privacy-friendly analytics service (" a href="https://plausible.io/" { "Plausible" } ") that does not track personal information. It's the only loaded javascript and it's loaded asynchronously to not block the page load." }
                li { "open-source: the code is available on " a href="https://github.com/RCasatta/fbbe" { "GitHub" } "." }
                li { "high-performance: the site achieves perfect 100 scores on all " a href="https://pagespeed.web.dev/analysis/https-fbbe-info/br6pboz6h5?form_factor=desktop" { "PageSpeed Insights" } " metrics." }
            }
        }
    };

    Ok(html_page("About", content, parsed))
}
