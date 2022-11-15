use std::{
    io::{self, Write},
    iter,
};

use bitcoin::{consensus::Decodable, Transaction, Txid};
use bitcoin_hashes::hex::FromHex;
use futures::{future, prelude::*, stream::FuturesUnordered, StreamExt};
use hyper::{
    body::{self, Buf},
    Client,
};
use thousands::{digits, Separable, SeparatorPolicy};

#[test]
fn test_separator() {
    let policy = SeparatorPolicy {
        separator: " ", // NARROW NO-BREAK SPACE' (U+202F)
        groups: &[3],
        digits: digits::ASCII_DECIMAL,
    };

    assert_eq!(
        1234567890.separate_by_policy(policy),
        "1\u{202f}234\u{202f}567\u{202f}890"
    );
}

#[tokio::test]
async fn test_unordered() {
    let mut futures = FuturesUnordered::new();
    futures.push(future::ready(1));
    let result = futures.next().await.unwrap();
    assert_eq!(result, 1);
}

const N_CONCURRENT: usize = 2000;

#[tokio::test]
async fn test_buffer_unordered() {
    let client = Client::new();
    let bitcoind_socket = "10.0.0.2:8332";

    let txid =
        Txid::from_hex("52539a56b1eb890504b775171923430f0355eb836a57134ba598170a2f8980c1").unwrap();

    let uri = format!("http://{bitcoind_socket}/rest/tx/{txid}.bin")
        .parse()
        .unwrap();
    let resp = client.get(uri).await.unwrap();

    let body_bytes = hyper::body::to_bytes(resp.into_body()).await.unwrap();
    let tx = Transaction::consensus_decode(&mut body_bytes.reader()).unwrap();

    assert_eq!(tx.input.len(), 20_000);

    stream::iter(tx.input.iter())
        .map(move |input| {
            let uri = format!(
                "http://{bitcoind_socket}/rest/tx/{}.bin",
                input.previous_output.txid
            );
            client.get(uri.parse().unwrap())
        })
        .buffer_unordered(N_CONCURRENT)
        .then(|res| async {
            let res = res.expect("Error making request: {}");
            let status = res.status();
            body::to_bytes(res).await.expect("Error reading body");
            status
        })
        .for_each(|status| async move { assert!(status.is_success()) })
        .await;
}
