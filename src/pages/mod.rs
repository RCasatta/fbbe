use std::collections::BTreeSet;

use crate::{globals::networks, network, render::SizeRow, NetworkExt};
use bitcoin::Network;
use maud::{html, Markup, PreEscaped, DOCTYPE};

pub mod address;
pub mod block;
pub mod contact;
pub mod home;
pub mod tx;
pub mod txout;

pub const NBSP: PreEscaped<&str> = PreEscaped("&nbsp;");

/// A basic header with a dynamic `page_title`.
pub fn header(title: &str) -> Markup {
    html! {
        head {
            meta charset="utf-8";
            meta name="viewport" content="width=device-width, initial-scale=1";
            meta name="description" content="A Fast Bitcoin Block Explorer: simple, bitcoin-only, cache-friendly, terminal-friendly, low-bandwith, no images, no javascript. With mainnet, testnet and signet.";
            link rel="stylesheet" href="/css/pico.min.css";
            style { (include_str!("../css/custom.min.css")) }
            title { "FBBE - "(title) }
        }
    }
}

fn nav_header() -> Markup {
    let title = match network() {
        Network::Bitcoin => "Fast Bitcoin Block Explorer",
        Network::Testnet => "Fast Bitcoin Block Explorer (Testnet)",
        Network::Signet => "Fast Bitcoin Block Explorer (Signet)",
        Network::Regtest => "Fast Bitcoin Block Explorer (Regtest)",
    };

    let mut other_networks: BTreeSet<_> = networks().into_iter().collect();
    other_networks.remove(&network());

    html! {
        nav {
            ul {
                li { a href=(network().as_url_path()) { (title) } }
            }

            @if !other_networks.is_empty() {
                ul {
                    @for net in other_networks {
                        li { a href=(net.as_url_path()) { (net.to_maiusc_string()) } }
                    }
                }
            }
        }
    }
}

pub fn html_page(title: &str, content: Markup) -> Markup {
    html! {
        (DOCTYPE)
        html lang = "en" {
            (header(title))
            body {
                div class="container" {
                    (nav_header())
                }
                main class="container" {
                    (content)
                }
                (footer())
            }
        }
    }
}

/// A static footer.
pub fn footer() -> Markup {
    let link = match network() {
        Network::Bitcoin => "/",
        Network::Testnet => "/testnet/",
        Network::Signet => "/signet/",
        Network::Regtest => "/regtest/",
    };
    html! {
        footer {
            div class="container" {
                a href=(link) { "Home" }
                " | " a href="/contact" { "Contact" }
                " | " a href="https://github.com/RCasatta/fbbe" { "Source" }

            }
        }
    }
}

pub fn size_rows(size: usize, weight: usize) -> Markup {
    let vsize = (weight + 3) / 4;

    html! {
        (SizeRow::new("Size (B)", size))
        (SizeRow::new("Virtual size (vB)", vsize))
        (SizeRow::new("Weight units (WU)", weight))
    }
}
