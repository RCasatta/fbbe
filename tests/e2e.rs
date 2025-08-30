use bitcoin::Network;
use bitcoind::{bitcoincore_rpc::RpcApi, BitcoinD, Conf};
use env_logger::Env;
use fbbe::create_local_socket;
use nix::{
    sys::signal::{self, Signal},
    unistd::Pid,
};
use std::{net::SocketAddr, path::Path, process::Child, str::from_utf8, time::Duration};

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

fn init_fbbe(bitcoind: &BitcoinD, network: Network) -> (SocketAddr, String, Vec<String>) {
    let exe = {
        let debug = "./target/debug/fbbe";
        let release = "./target/release/fbbe";
        if Path::new(debug).exists() {
            debug.to_string()
        } else if Path::new(debug).exists() {
            release.to_string()
        } else {
            let env = std::env::var("FBBE_EXE").expect("specify `fbbe` executable in FBBE_EXE env");
            if Path::new(&env).exists() {
                env
            } else {
                panic!("env var FBBE_EXE is pointing to non existing file");
            }
        }
    };

    let fbbe_addr = create_local_socket(bitcoind::get_available_port().unwrap());
    let args = vec![
        "--bitcoind-addr".into(),
        bitcoind.params.rpc_socket.to_string(),
        "--network".into(),
        network.to_string(),
        "--local-addr".into(),
        fbbe_addr.to_string(),
    ];
    (fbbe_addr, exe, args)
}

struct FbbeProcess(Child);

impl Drop for FbbeProcess {
    fn drop(&mut self) {
        let _ = signal::kill(Pid::from_raw(self.0.id() as i32), Signal::SIGINT);
    }
}

impl FbbeProcess {
    fn new(exe: String, args: Vec<String>) -> Self {
        let child = std::process::Command::new(exe).args(args).spawn().unwrap();
        // TODO wait until online
        std::thread::sleep(Duration::from_secs(1));
        Self(child)
    }
}

#[test]
fn check_pages() {
    let bitcoind = init_node();

    let (fbbe_addr, exe, args) = init_fbbe(&bitcoind, Network::Regtest);

    let _fbbe_proc = FbbeProcess::new(exe, args);

    let get = |url: String| {
        minreq::get(url)
            .send()
            .unwrap()
            .as_str()
            .unwrap()
            .to_string()
    };

    let home_page = format!("http://{fbbe_addr}");
    let page = get(home_page);
    assert!(page.contains("Fast Bitcoin Block Explorer"));

    // ensure the input it's there with the right name
    let document = scraper::Html::parse_document(&page);
    let selector = scraper::Selector::parse("input").unwrap();
    let mut iter = document.select(&selector);
    let element = iter.next().unwrap();
    assert_eq!("s", format!("{}", element.value().attr("name").unwrap()));
    assert!(iter.next().is_none());

    let genesis_block = "0f9188f13cb7b2c71f2a335e3a4fc328bf5beb436012afca590b1a11466e2206";
    let genesis_tx = "4a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b";

    let block_page = format!("http://{fbbe_addr}/b/{genesis_block}");
    let page = get(block_page);
    assert!(page.contains(genesis_tx));
    assert!(page.contains(genesis_block));

    let block_page_search = format!("http://{fbbe_addr}/?s=0");
    let page_from_search = get(block_page_search);
    assert_eq!(page, page_from_search);

    let tx_page = format!("http://{fbbe_addr}/t/{genesis_tx}");
    let page = get(tx_page);
    assert!(page.contains(genesis_block));
    assert!(page.contains(genesis_tx));
}
#[test]
fn check_wrong_network() {
    let bitcoind = init_node();
    let (_, exe, args) = init_fbbe(&bitcoind, Network::Testnet);

    let output = std::process::Command::new(exe)
        .args(args)
        .output()
        .unwrap()
        .stderr;

    let s = from_utf8(&output).unwrap();
    assert!(s.contains(
        "bitcoind and fbbe doesn't have the same network. fbbe:testnet bitcoind:regtest"
    ));
}
