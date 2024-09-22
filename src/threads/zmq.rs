use std::net::SocketAddr;

use async_zmq::{subscribe, Context};
use futures::StreamExt;

pub async fn update_tx_zmq(socket: &SocketAddr) {
    log::info!("Start update_tx_zmq!");

    let context = Context::new();
    let url = format!("tcp://{socket}");

    let mut sub = subscribe(&url)
        .unwrap()
        .with_context(&context)
        .connect()
        .unwrap();
    sub.set_subscribe("rawtx").unwrap();

    while let Some(msg) = sub.next().await {
        // Received message is a type of Result<MessageBuf>
        let msg = msg.unwrap();

        log::info!("{:?}", msg.iter());
    }
}
