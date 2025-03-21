use std::{env, path::PathBuf};

use electrsd::{
    bitcoind::{
        bitcoincore_rpc::{jsonrpc::serde_json::Value, RpcApi},
        BitcoinD, P2P,
    },
    ElectrsD,
};
use joinstr::{electrum::Client, signer::WpkhHotSigner};
use miniscript::bitcoin::{Address, Amount, Network};

pub fn bootstrap_electrs() -> (
    String, /* url */
    u16,    /* port */
    ElectrsD,
    BitcoinD,
) {
    let mut cwd: PathBuf = env::current_dir().expect("Failed to get current directory");
    cwd.push("tests");

    let mut electrs_path = cwd.clone();
    electrs_path.push("bin");
    electrs_path.push("electrs_0_9_11");

    let mut bitcoind_path = cwd.clone();
    bitcoind_path.push("bin");
    bitcoind_path.push("bitcoind_25_2");

    let mut conf = electrsd::bitcoind::Conf::default();
    conf.p2p = P2P::Yes;
    let bitcoind = BitcoinD::with_conf(bitcoind_path, &conf).unwrap();

    let mut electrsd_conf = electrsd::Conf::default();
    electrsd_conf.args = vec!["--log-filters", "DEBUG"];
    electrsd_conf.buffered_logs = true;

    let electrsd = ElectrsD::with_conf(electrs_path, &bitcoind, &electrsd_conf).unwrap();
    let (url, port) = electrsd.electrum_url.split_once(':').unwrap();
    let port = port.parse::<u16>().unwrap();

    // mine 101 blocks
    let node_address = bitcoind.client.call::<Value>("getnewaddress", &[]).unwrap();
    bitcoind
        .client
        .call::<Value>("generatetoaddress", &[101.into(), node_address])
        .unwrap();

    (url.into(), port, electrsd, bitcoind)
}

pub fn tcp_client() -> (Client, ElectrsD, BitcoinD) {
    let (url, port, e, b) = bootstrap_electrs();
    let client = Client::new(&url, port).unwrap();

    (client, e, b)
}

pub fn send_to_address(bitcoind: &BitcoinD, addr: &Address, amount: Amount) {
    let txid = bitcoind
        .client
        .send_to_address(addr, amount, None, None, None, None, None, None)
        .unwrap();
    log::debug!("send_to_address({}, {}) => {}", addr, amount, txid);
}

pub fn generate(bitcoind: &BitcoinD, blocks: u32) {
    let node_address = bitcoind.client.call::<Value>("getnewaddress", &[]).unwrap();
    bitcoind
        .client
        .call::<Value>("generatetoaddress", &[blocks.into(), node_address])
        .unwrap();
}

pub fn funded_wallet(amounts: &[f64]) -> (WpkhHotSigner, Client, ElectrsD, BitcoinD) {
    let (client, electrsd, bitcoind) = tcp_client();
    let signer = WpkhHotSigner::new(Network::Regtest).unwrap();
    for (i, a) in amounts.iter().enumerate() {
        let addr = signer.recv_addr_at(i as u32);
        let amount = Amount::from_btc(*a).unwrap();
        send_to_address(&bitcoind, &addr, amount);
    }
    generate(&bitcoind, 2);
    (signer, client, electrsd, bitcoind)
}

pub fn funded_wallet_with_bitcoind(amounts: &[f64], bitcoind: &BitcoinD) -> WpkhHotSigner {
    let signer = WpkhHotSigner::new(Network::Regtest).unwrap();
    for (i, a) in amounts.iter().enumerate() {
        let addr = signer.recv_addr_at(i as u32);
        let amount = Amount::from_btc(*a).unwrap();
        send_to_address(bitcoind, &addr, amount);
    }
    generate(bitcoind, 10);
    signer
}
