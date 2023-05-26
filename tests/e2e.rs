use std::{net::SocketAddr, str::from_utf8, time::Duration};

use bitcoin::Network;
use bitcoind::{bitcoincore_rpc::RpcApi, BitcoinD, Conf};
use clap::Parser;
use env_logger::Env;
use fbbe::{create_local_socket, Arguments};

fn init_node() -> BitcoinD {
    let _ = env_logger::Builder::from_env(Env::default()).try_init();
    let mut config = Conf::default();

    config.args.push("-rest");

    let path = bitcoind::exe_path();
    let bitcoind = bitcoind::BitcoinD::with_conf(path.unwrap(), &config).unwrap();

    let addr = bitcoind
        .client
        .get_new_address(None, None)
        .unwrap()
        .assume_checked();
    let _blocks = bitcoind.client.generate_to_address(1, &addr).unwrap();
    bitcoind
}

fn init_fbbe(bitcoind: &BitcoinD, network: Network) -> Arguments {
    let mut args = Arguments::parse_from(Vec::<String>::new());
    args.bitcoind_addr = Some(bitcoind.params.rpc_socket.into());
    args.network = Some(network);
    let fbbe_addr = create_local_socket(bitcoind::get_available_port().unwrap());
    args.local_addr = Some(fbbe_addr);
    args
}

fn fbbe_args(bitcoind: &BitcoinD, network: Network) -> Vec<String> {
    let fbbe_addr = create_local_socket(bitcoind::get_available_port().unwrap());
    let mut args = vec![];
    args.push("--bitcoind-addr".into());
    args.push(bitcoind.params.rpc_socket.to_string());
    args.push("--network".to_string());
    args.push(network.to_string());
    args.push("--local-addr".to_string());
    args.push(fbbe_addr.to_string());
    args
}

#[test]
fn check_pages() {
    let bitcoind = init_node();

    let args = init_fbbe(&bitcoind, Network::Regtest);
    let fbbe_addr = args.local_addr.clone().unwrap();

    // TODO shutdown the thread
    let _h = std::thread::spawn(|| {
        // TODO launching this way, only one fbbe per test run can be launched because of globals
        // consider migrating to explicit process also here
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async { fbbe::inner_main(args).await })
            .unwrap();
    });

    // TODO wait until online
    std::thread::sleep(Duration::from_secs(1));

    let get = |url: String| {
        minreq::get(&url)
            .send()
            .unwrap()
            .as_str()
            .unwrap()
            .to_string()
    };

    let home_page = format!("http://{fbbe_addr}");
    let page = get(home_page);
    assert!(page.contains("Fast Bitcoin Block Explorer"));

    let genesis_block = "0f9188f13cb7b2c71f2a335e3a4fc328bf5beb436012afca590b1a11466e2206";
    let genesis_tx = "4a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b";

    let block_page = format!("http://{fbbe_addr}/b/{genesis_block}");
    let page = get(block_page);
    assert!(page.contains(genesis_tx));
    assert!(page.contains(genesis_block));

    let tx_page = format!("http://{fbbe_addr}/t/{genesis_tx}");
    let page = get(tx_page);
    assert!(page.contains(genesis_block));
    assert!(page.contains(genesis_tx));
}

#[test]
fn check_wrong_network() {
    let bitcoind = init_node();
    let args = fbbe_args(&bitcoind, Network::Testnet);

    let output = std::process::Command::new("./target/debug/fbbe")
        .args(args)
        .output()
        .unwrap()
        .stderr;

    let s = from_utf8(&output).unwrap();
    assert!(s.contains(
        "bitcoind and fbbe doesn't have the same network. fbbe:testnet bitcoind:regtest"
    ));
}
