use std::time::Duration;

use bitcoin::Network;
use bitcoind::{bitcoincore_rpc::RpcApi, Conf};
use clap::Parser;
use env_logger::Env;
use fbbe::{create_local_socket, Arguments};

#[ignore] // requires bitcoind dep, in nix cannot autodownload the executable
#[test]
fn check_pages() {
    env_logger::Builder::from_env(Env::default()).init();
    let mut config = Conf::default();

    config.args.push("-rest");

    let bitcoind =
        bitcoind::BitcoinD::with_conf(bitcoind::downloaded_exe_path().unwrap(), &config).unwrap();
    let addr = bitcoind
        .client
        .get_new_address(None, None)
        .unwrap()
        .assume_checked();
    let _blocks = bitcoind.client.generate_to_address(1, &addr).unwrap();

    let mut args = Arguments::parse_from(Vec::<String>::new());
    args.bitcoind_addr = Some(bitcoind.params.rpc_socket.into());
    args.network = Some(Network::Regtest);
    let fbbe_addr = create_local_socket(bitcoind::get_available_port().unwrap());
    args.local_addr = Some(fbbe_addr);

    // TODO shutdown the thread
    let _h = std::thread::spawn(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async { fbbe::inner_main(args).await })
            .unwrap();
    });

    // TODO wait until online
    std::thread::sleep(Duration::from_secs(1));

    let get = |url: String| ureq::get(&url).call().unwrap().into_string().unwrap();

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
