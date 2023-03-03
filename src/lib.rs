use crate::error::Error;
use crate::globals::{init_globals, network};
use crate::route::route_infallible;
use crate::state::SharedState;
use crate::threads::bootstrap_state::bootstrap_state_infallible;
use crate::threads::update_chain_info::update_chain_info_infallible;
use crate::threads::update_mempool_info::update_mempool;
use bitcoin::Network;
use globals::networks;
use hyper::service::{make_service_fn, service_fn};
use hyper::Server;
use std::convert::Infallible;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use structopt::StructOpt;
use tokio::time::sleep;

mod error;
mod globals;
mod pages;
mod render;
mod req;
mod route;
mod rpc;
mod state;
mod threads;

#[derive(StructOpt)]
pub struct Arguments {
    /// Number of transaction kept in memory in a least recently used cache to reduce the number of
    /// requests of transactions to bitcoin core
    #[structopt(short, long, default_value = "100000")]
    pub tx_cache_size: usize,

    /// Some requests to the bitcoin core are concurrent, this set the desired parallelism.
    /// Note there is a limit of open files that this setting too high could trigger.
    /// See https://github.com/bitcoin/bitcoin/blob/master/doc/REST-interface.md#risks
    #[structopt(short, long, default_value = "10")]
    pub fetch_parallelism: usize,

    /// default to "127.0.0.1:<port>" where port depend on the network used, eg 8332 for mainnnet.
    #[structopt(short, long)]
    pub bitcoind_addr: Option<SocketAddr>,

    /// default value: bitcoin
    /// option so that is consumed with take when passed to `NETWORK` global var
    #[structopt(short, long)]
    pub network: Option<Network>,

    /// The socket address this service will bind on. Default value depends on the network:
    /// * mainnet: "127.0.0.1:3000"
    /// * testnet: "127.0.0.1:3001"
    /// * signet:  "127.0.0.1:3002"
    /// * regtest: "127.0.0.1:3003"

    #[structopt(short, long)]
    pub local_addr: Option<SocketAddr>,

    /// If the setup involve multiple networks this must be set accordingly.
    /// An header with a link to all the network is generated.
    /// Links are prepended the network if it isn't mainet (eg `/testnet/t/xxx...`)
    /// Note the routes are still working without the network, it is duty of a frontend to redirect the
    /// path to appropriate port. eg.
    ///
    ///   ```no_build
    ///   location = /testnet {
    ///     return 302 /testnet/;
    ///   }
    ///   location /testnet/ {
    ///     proxy_pass http://10.0.0.7:3001/;
    ///   }
    ///   ```
    ///
    #[structopt(short, long)]
    pub other_network: Vec<Network>,
}

pub async fn inner_main(mut args: Arguments) -> Result<(), Error> {
    init_globals(&mut args);

    let addr = args.local_addr.take().unwrap_or_else(|| match network() {
        Network::Bitcoin => create_local_socket(3000),
        Network::Testnet => create_local_socket(3001),
        Network::Signet => create_local_socket(3002),
        Network::Regtest => create_local_socket(3003),
    });
    let args = args;

    log::debug!("local address {:?}", addr);

    let mut chain_info;
    loop {
        chain_info = match rpc::chaininfo::call().await {
            Ok(chain_info) => chain_info,
            Err(Error::RpcChainInfo(status_code)) if status_code == 404 => {
                return Err(Error::RestFlag);
            }
            Err(Error::RpcChainInfo(status_code)) if status_code == 503 => {
                log::warn!("bitcoind is still loading, waiting... (note: if on regtest you may need to generate a block to terminate IBD)");
                sleep(tokio::time::Duration::from_secs(10)).await;
                continue;
            }
            Err(e) => {
                let network = network();
                log::error!(
                    "bitcoind is probably not running, or running on wrong network {network}",
                );
                return Err(e);
            }
        };
        if chain_info.initial_block_download {
            log::warn!("bitcoind is not synced, waiting (on regtest you may need to generate a block)... {:?}", chain_info);
            sleep(tokio::time::Duration::from_secs(10)).await;
        } else {
            log::info!("bitcoind is synced: {:?}", chain_info);
            break;
        }
    }

    match chain_info.chain.as_str() {
        "main" => check_network(Network::Bitcoin)?,
        "test" => check_network(Network::Testnet)?,
        "signet" => check_network(Network::Signet)?,
        "regtest" => check_network(Network::Regtest)?,
        net => panic!("Network returned by bitcoind is not supported: {}", net),
    }

    let mempool_info = rpc::mempool::info().await?;
    log::info!("{:?}", mempool_info);

    let shared_state = Arc::new(SharedState::new(chain_info.clone(), args, mempool_info));

    // initialize cache with information from headers
    let shared_state_bootstrap = shared_state.clone();
    let h = tokio::spawn(async move { bootstrap_state_infallible(shared_state_bootstrap).await });

    // keep chain info updated
    let shared_state_chain = shared_state.clone();
    let shared_state_mempool = shared_state.clone();

    let _ = tokio::spawn(async move {
        h.await.unwrap();
        let _ = tokio::spawn(async move {
            update_chain_info_infallible(shared_state_chain, chain_info).await
        });
        update_mempool(shared_state_mempool).await
    });

    let make_service = make_service_fn(move |_| {
        let shared_state = shared_state.clone();

        async move {
            Ok::<_, Infallible>(service_fn(move |req| {
                let shared_state = shared_state.clone();
                route_infallible(req, shared_state)
            }))
        }
    });

    let server = Server::bind(&addr).serve(make_service);

    log::info!("Listening on http://{}", addr);

    if let Err(e) = server.await {
        log::error!("server error: {}", e);
    }
    Ok(())
}

fn check_network(bitcoind: Network) -> Result<(), Error> {
    let fbbe = network();

    (fbbe == bitcoind)
        .then_some(())
        .ok_or(Error::WrongNetwork { fbbe, bitcoind })
}

trait NetworkExt {
    fn as_url_path(&self) -> String;
    fn to_maiusc_string(&self) -> String;
}

impl NetworkExt for Network {
    fn as_url_path(&self) -> String {
        if let Network::Bitcoin = self {
            "/".to_string()
        } else if networks().len() == 1 {
            "/".to_string()
        } else {
            format!("/{}/", self)
        }
    }

    fn to_maiusc_string(&self) -> String {
        format!("{:?}", self)
    }
}

pub fn create_local_socket(port: u16) -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), port)
}
