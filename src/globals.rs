use bitcoin::Network;
use once_cell::sync::OnceCell;
use std::{collections::HashSet, net::SocketAddr};

use crate::{create_local_socket, Arguments};

static NETWORK: OnceCell<Network> = OnceCell::new();

pub(crate) fn network() -> Network {
    *NETWORK.get().expect("must be initialized")
}

static BITCOIND_ADDR: OnceCell<SocketAddr> = OnceCell::new();

pub(crate) fn bitcoind_addr() -> &'static SocketAddr {
    BITCOIND_ADDR.get().expect("must be initialized")
}

static NETWORKS: OnceCell<Vec<Network>> = OnceCell::new();

pub(crate) fn networks() -> &'static [Network] {
    NETWORKS.get().expect("must be initialized")
}

pub(crate) fn init_globals(args: &mut Arguments) {
    NETWORK
        .set(
            args.network
                .take()
                .map(Into::into)
                .unwrap_or(Network::Bitcoin),
        )
        .expect("static global must be empty here");

    let mut networks = HashSet::new();
    networks.insert(network());
    networks.extend(args.other_network.iter());
    let networks: Vec<_> = networks.into_iter().collect();
    log::info!("networks {:?}", networks);

    NETWORKS
        .set(networks)
        .expect("static global must be empty here");

    let bitcoind_addr = args.bitcoind_addr.take().unwrap_or_else(|| {
        let port = match network() {
            Network::Bitcoin => 8332,
            Network::Testnet => 18332,
            Network::Signet => 38332,
            Network::Regtest => 18443,
            _ => panic!("non existing network"),
        };
        create_local_socket(port)
    });
    log::info!("bitcoind_addr {}", bitcoind_addr);
    BITCOIND_ADDR
        .set(bitcoind_addr)
        .expect("static global must be empty here");
}
