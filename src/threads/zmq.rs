use std::{net::SocketAddr, sync::Arc};

use async_zmq::{subscribe, Context};
use bitcoin::{hashes::Hash, Txid};
use bitcoin_slices::{bsl, Parse};
use futures::StreamExt;

use crate::{state::SharedState, Error};

pub async fn update_tx_zmq_infallible(socket: &SocketAddr, state: Arc<SharedState>) {
    if let Err(e) = update_tx_zmq(socket, state).await {
        log::error!("{:?}", e);
    }
}

async fn update_tx_zmq(socket: &SocketAddr, state: Arc<SharedState>) -> Result<(), Error> {
    log::info!("Start update_tx_zmq!");

    let context = Context::new();
    let url = format!("tcp://{socket}");

    let mut sub = subscribe(&url).unwrap().with_context(&context).connect()?;
    sub.set_subscribe("rawtx")?;
    let mut count = 0u64;

    while let Some(msg) = sub.next().await {
        let msg = msg.unwrap();
        // | "rawtx" | <serialized transaction> | <uint32 sequence number in Little Endian>
        if let Some(tx) = msg.get(1) {
            if let Ok(tx) = bsl::Transaction::parse(tx) {
                count += 1;

                let txid = tx.parsed().txid_sha2();
                let txid = Txid::from_byte_array(txid.into());

                // TODO load also prevouts?

                let insert_result = state.txs.lock().await.insert(txid, tx.parsed());
                log::trace!("inserting {} {}", txid, insert_result.is_ok());
            }
        }
        if count % 10_000 == 0 {
            let (cache_len, cache_avg) = {
                let c = state.txs.lock().await;
                (c.len(), c.avg())
            };
            log::info!("zmq received {count} txs, (cache len: {cache_len}, avg: {cache_avg:.1})");
        }
    }
    Ok(())
}
