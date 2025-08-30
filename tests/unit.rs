use std::{io::BufReader, str::FromStr};

use bitcoin::{consensus::Decodable, Transaction, Txid};
use futures::{future, prelude::*, stream::FuturesUnordered, StreamExt};
use hyper::body::Buf;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;

#[tokio::test]
async fn test_unordered() {
    let mut futures = FuturesUnordered::new();
    futures.push(future::ready(1));
    let result = futures.next().await.unwrap();
    assert_eq!(result, 1);
}

const N_CONCURRENT: usize = 2000;

#[ignore]
#[tokio::test]
async fn test_buffer_unordered() {
    let client: Client<_, http_body_util::Full<hyper::body::Bytes>> =
        Client::builder(TokioExecutor::new()).build_http();
    let bitcoind_socket = "10.0.0.2:8332";

    let txid =
        Txid::from_str("52539a56b1eb890504b775171923430f0355eb836a57134ba598170a2f8980c1").unwrap();

    let uri = format!("http://{bitcoind_socket}/rest/tx/{txid}.bin")
        .parse()
        .unwrap();
    let resp = client.get(uri).await.unwrap();

    let body_bytes = http_body_util::BodyExt::collect(resp.into_body())
        .await
        .unwrap()
        .to_bytes();
    let mut reader = BufReader::new(body_bytes.reader());
    let tx = Transaction::consensus_decode(&mut reader).unwrap();

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
            http_body_util::BodyExt::collect(res.into_body())
                .await
                .expect("Error reading body");
            status
        })
        .for_each(|status| async move { assert!(status.is_success()) })
        .await;
}
