use std::collections::BTreeSet;

use crate::{
    globals::networks,
    network,
    render::SizeRow,
    req::{ParsedRequest, Resource},
    route::ResponseType,
    NetworkExt,
};
use bitcoin::Network;
use maud::{html, Markup, PreEscaped, DOCTYPE};

pub mod about;
pub mod address;
pub mod block;
pub mod home;
pub mod tx;

pub const NBSP: PreEscaped<&str> = PreEscaped("&nbsp;");

/// A basic header with a dynamic `page_title`.
pub fn header(title: &str) -> Markup {
    html! {
        head {
            meta charset="utf-8";
            meta name="viewport" content="width=device-width, initial-scale=1";
            meta name="description" content="A Fast Bitcoin Block Explorer: simple, bitcoin-only, cache-friendly, terminal-friendly, low-bandwith, no images, no javascript. With mainnet and signet.";
            link rel="preload" href="/css/pico.min.css" as="style";
            link rel="stylesheet" href="/css/pico.min.css";
            style { (include_str!("../css/custom.min.css")) }
            script defer data-domain="fbbe.info" src="https://plausible.casatta.it/js/script.js" {}
            title { "FBBE - "(title) }
        }
    }
}

fn nav_header(response_type: ResponseType) -> Markup {
    let title = match network() {
        Network::Bitcoin => "Fast Bitcoin Block Explorer",
        Network::Testnet => "Fast Bitcoin Block Explorer (Testnet)",
        Network::Signet => "Fast Bitcoin Block Explorer (Signet)",
        Network::Regtest => "Fast Bitcoin Block Explorer (Regtest)",
        _ => panic!("non existing network"),
    };

    let mut other_networks: BTreeSet<_> = networks().iter().collect();
    other_networks.remove(&network());

    html! {
        nav {
            ul {
                li { a href=(network().as_url_path()) aria-current="page" { (title) } }
            }

            @if !other_networks.is_empty() && !response_type.is_text() {
                ul {
                    @for net in other_networks {
                        li { a href=(net.as_url_path()) { (net.to_maiusc_string()) } }
                    }
                }
            }
        }
    }
}

pub fn html_page(title: &str, content: Markup, parsed: &ParsedRequest) -> Markup {
    html! {
        (DOCTYPE)
        html lang = "en" {
            (header(title))
            body {
                div class="container" {
                    (nav_header(parsed.response_type))
                }
                main class="container" {
                    (content)
                }
                (footer(parsed))
            }
        }
    }
}

/// A static footer.
pub fn footer(parsed: &ParsedRequest) -> Markup {
    if parsed.response_type.is_text() {
        return html! {};
    }
    let base = network().as_url_path();

    let home = if let Resource::Home = parsed.resource {
        html! { a href=(base) aria-current="page" { "Home" } }
    } else {
        html! { a href=(base) { "Home" } }
    };
    html! {
        footer {
            div class="container" {
                (home)
                @if let Some(link) = parsed.resource.link() {
                    " | " a href=(link) { "Text" }
                }
                " | " a href="/about" { "About" }
                " | " a href="https://github.com/RCasatta/fbbe" { "Source" }

            }
        }
    }
}

pub fn size_rows(size: usize, weight: usize) -> Markup {
    let vsize = (weight + 3) / 4;

    html! {
        (SizeRow::new("Size", size as u64))
        (SizeRow::new("Virtual size", vsize as u64))
    }
}
