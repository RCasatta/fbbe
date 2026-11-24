use bitcoin::Network;
use bitcoind::{BitcoinD, Conf};
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
    config.args.push("-txindex");
    // No wallet: wallet RPC responses change across Core versions and break typed clients
    // when the node (eg. BITCOIND_EXE from nix) is newer than the selected feature.
    config.wallet = None;

    let path = bitcoind::exe_path();
    let bitcoind = bitcoind::BitcoinD::with_conf(path.unwrap(), &config).unwrap();

    // P2WSH(OP_TRUE): anyone-can-spend address, no keys or wallet needed
    let script = bitcoin::ScriptBuf::from_bytes(vec![bitcoin::opcodes::OP_TRUE.to_u8()]);
    let addr = bitcoin::Address::p2wsh(&script, Network::Regtest);
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
    assert!(page.contains(&format!("/t/{genesis_tx}/hex")));

    let tx_hex = get(format!("http://{fbbe_addr}/t/{genesis_tx}/hex"));
    let tx: bitcoin::Transaction = bitcoin::consensus::encode::deserialize_hex(&tx_hex).unwrap();
    assert_eq!(tx.compute_txid().to_string(), genesis_tx);

    // redirects keep the requested text format
    let block_text = get(format!("http://{fbbe_addr}/b/{genesis_block}/text"));
    assert!(block_text.starts_with("* Fast Bitcoin Block Explorer"));
    assert!(block_text.contains("Coinbase value (BTC) 50.00000000\n"));
    assert!(block_text.contains("Fees (BTC)           0.00000000\n"));
    assert!(block_text.contains(" 285 B\n"));
    assert!(block_text.contains("Next block »"));
    assert_eq!(block_text, get(format!("http://{fbbe_addr}/text?s=0")));
    assert_eq!(block_text, get(format!("http://{fbbe_addr}/h/0/text")));
    assert_eq!(
        get(format!("http://{fbbe_addr}/t/{genesis_tx}/text")),
        get(format!("http://{fbbe_addr}/text?s={genesis_tx}"))
    );

    // agents asking for text/plain or text/markdown get the text page
    let tx_text = get(format!("http://{fbbe_addr}/t/{genesis_tx}/text"));
    let resp = minreq::get(format!("http://{fbbe_addr}/t/{genesis_tx}"))
        .with_header("Accept", "text/plain")
        .send()
        .unwrap();
    assert_eq!(resp.as_str().unwrap(), tx_text);
    assert_eq!(resp.headers.get("vary").unwrap(), "Accept, columns");
    let resp = minreq::get(format!("http://{fbbe_addr}/t/{genesis_tx}"))
        .with_header("Accept", "text/markdown")
        .send()
        .unwrap();
    assert_eq!(resp.as_str().unwrap(), tx_text);

    // text width depends on the columns header, caches must know it
    let resp = minreq::get(format!("http://{fbbe_addr}/t/{genesis_tx}/text"))
        .with_header("columns", "40")
        .send()
        .unwrap();
    assert_ne!(resp.as_str().unwrap(), tx_text);
    assert_eq!(resp.headers.get("vary").unwrap(), "columns");

    // no ASCII QR code in text
    let script = bitcoin::ScriptBuf::from_bytes(vec![bitcoin::opcodes::OP_TRUE.to_u8()]);
    let addr = bitcoin::Address::p2wsh(&script, Network::Regtest);
    let address_text = get(format!("http://{fbbe_addr}/a/{addr}/text"));
    assert!(address_text.contains(&addr.to_string()));
    assert!(!address_text.contains('█'));

    let about = get(format!("http://{fbbe_addr}/about/text"));
    assert!(about.starts_with("* Fast Bitcoin Block Explorer"));

    // missing resources are 404, invalid input is 400
    let status = |url: String| minreq::get(url).send().unwrap();
    let missing_tx = "00000000000000000000000000000000000000000000000000000000deadbeef";
    let resp = status(format!("http://{fbbe_addr}/t/{missing_tx}/text"));
    assert_eq!(resp.status_code, 404);
    assert_eq!(
        resp.as_str().unwrap(),
        format!("Transaction {missing_tx} not found\n")
    );
    let missing_block = "000000000000000000000000000000000000000000000000000000000000beef";
    assert_eq!(
        status(format!("http://{fbbe_addr}/b/{missing_block}/text")).status_code,
        404
    );
    let resp = status(format!("http://{fbbe_addr}/h/999999"));
    assert_eq!(resp.status_code, 404);
    assert_eq!(resp.as_str().unwrap(), "Block at height 999999 not found\n");
    let resp = status(format!("http://{fbbe_addr}/a/tb1qxyz/text"));
    assert_eq!(resp.status_code, 400);
    assert_eq!(
        resp.as_str().unwrap(),
        "Bad request: invalid address tb1qxyz\n"
    );
    let resp = status(format!("http://{fbbe_addr}/text?s=garbage"));
    assert_eq!(resp.status_code, 400);
    assert!(resp
        .as_str()
        .unwrap()
        .starts_with("Bad request: search for a txid"));
    assert_eq!(
        status(format!("http://{fbbe_addr}/t/nothex/text")).status_code,
        400
    );
    assert_eq!(
        status(format!("http://{fbbe_addr}/a/notanaddress/text")).status_code,
        400
    );
}
#[test]
fn check_tx_text() {
    use bitcoin::{
        absolute::LockTime, consensus::encode::serialize_hex, transaction::Version, Amount,
        OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness,
    };

    let bitcoind = init_node();
    let script = ScriptBuf::from_bytes(vec![bitcoin::opcodes::OP_TRUE.to_u8()]);
    let addr = bitcoin::Address::p2wsh(&script, Network::Regtest);
    // coinbase of block 1 must mature
    let _blocks = bitcoind.client.generate_to_address(100, &addr).unwrap();
    let call = |method: &str, args: &[serde_json::Value]| {
        bitcoind
            .client
            .call::<serde_json::Value>(method, args)
            .unwrap()
    };
    let hash = call("getblockhash", &[1.into()]);
    let block_hex = call("getblock", &[hash, 0.into()]);
    let block: bitcoin::Block =
        bitcoin::consensus::encode::deserialize_hex(block_hex.as_str().unwrap()).unwrap();
    let coinbase = &block.txdata[0];
    let coinbase_txid = coinbase.compute_txid();

    let p2a = ScriptBuf::from_bytes(vec![0x51, 0x02, 0x4e, 0x73]);
    let tx = Transaction {
        version: Version::TWO,
        lock_time: LockTime::from_height(100).unwrap(),
        input: vec![TxIn {
            previous_output: OutPoint::new(coinbase_txid, 0),
            script_sig: ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness: Witness::from_slice(&[script.as_bytes()]),
        }],
        output: vec![
            TxOut {
                value: coinbase.output[0].value - Amount::from_sat(11_000),
                script_pubkey: addr.script_pubkey(),
            },
            TxOut {
                value: Amount::from_sat(1_000),
                script_pubkey: p2a,
            },
        ],
    };
    let txid = call("sendrawtransaction", &[serialize_hex(&tx).into()]);
    let txid = txid.as_str().unwrap();

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

    let text = get(format!("http://{fbbe_addr}/t/{txid}/text"));
    assert!(text.contains("Fee (BTC)"));
    assert!(text.contains("sat/vB)"));
    assert!(text.contains("0xfffffffd (signals RBF)"));
    assert!(text.contains("Signaled"));
    assert!(text.contains("100 (height 100)"));
    assert!(text.contains("Previous address "));
    assert!(text.contains(&format!("{addr}[")));
    assert!(text.contains("Script pubkey (v1 p2a)"));

    let coinbase_text = get(format!("http://{fbbe_addr}/t/{coinbase_txid}/text"));
    assert!(coinbase_text.contains("none (coinbase)"));
    assert!(coinbase_text.contains("Height in script sig (BIP34) 1\n"));

    let genesis_tx = "4a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b";
    let genesis_text = get(format!("http://{fbbe_addr}/t/{genesis_tx}/text"));
    assert!(genesis_text.contains("The Times 03/Jan/2009 Chancellor"));
    assert!(genesis_text.contains("(unspendable genesis output)"));
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
