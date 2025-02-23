pub mod utils;
use crate::utils::{funded_wallet, generate};

use electrsd::bitcoind::bitcoincore_rpc::RpcApi;
use joinstr::{coinjoin::CoinJoin, electrum::Client, signer::CoinPath};
use miniscript::bitcoin::Amount;

#[test]
fn simple_tx() {
    let (mut signer, mut client, _electrsd, bitcoind) =
        funded_wallet(&[0.11, 0.11, 0.11, 0.11, 0.11]);

    signer.set_client(client.clone());

    let coin = signer
        .get_coins_at(CoinPath {
            depth: 0,
            index: Some(0),
        })
        .unwrap();
    assert_eq!(coin, 1);

    // generate output addresses
    let addr0 = signer.recv_addr_at(100).unwrap();

    // prepare coinjoin template
    let mut coinjoin = CoinJoin::<Client>::new(Amount::from_btc(0.1).unwrap(), None)
        .min_peer(1)
        .output(addr0)
        .generate()
        .unwrap();
    let unsigned = coinjoin.unsigned_tx().unwrap();

    for coin in signer.list_coins() {
        let signed_input = signer.sign(&unsigned, coin.1).unwrap();
        coinjoin.add_input(signed_input).unwrap();
    }

    let finalyzed_tx = coinjoin.generate_tx(true).unwrap().unwrap();
    let txid = bitcoind.client.send_raw_transaction(&finalyzed_tx).unwrap();

    generate(&bitcoind, 10);

    let _tx = client.get_tx(txid).unwrap();
}

#[test]
fn simple_coinjoin() {
    let (mut signer, mut client, _electrsd, bitcoind) =
        funded_wallet(&[0.11, 0.11, 0.11, 0.11, 0.11]);

    signer.set_client(client.clone());

    // fetch input data for each coins
    (0..5).for_each(|i| {
        let coin = signer
            .get_coins_at(CoinPath {
                depth: 0,
                index: Some(i),
            })
            .unwrap();
        assert_eq!(coin, 1);
    });

    // generate output addresses
    let addr0 = signer.recv_addr_at(100).unwrap();
    let addr1 = signer.recv_addr_at(101).unwrap();
    let addr2 = signer.recv_addr_at(102).unwrap();
    let addr3 = signer.recv_addr_at(103).unwrap();
    let addr4 = signer.recv_addr_at(104).unwrap();

    // prepare coinjoin template
    let mut coinjoin = CoinJoin::<Client>::new(Amount::from_btc(0.1).unwrap(), None)
        .min_peer(5)
        .output(addr0)
        .output(addr1)
        .output(addr2)
        .output(addr3)
        .output(addr4)
        .generate()
        .unwrap();
    let unsigned = coinjoin.unsigned_tx().unwrap();

    for coin in signer.list_coins() {
        let signed_input = signer.sign(&unsigned, coin.1).unwrap();
        coinjoin.add_input(signed_input).unwrap();
    }

    let finalyzed_tx = coinjoin.generate_tx(true).unwrap().unwrap();
    let txid = bitcoind.client.send_raw_transaction(&finalyzed_tx).unwrap();

    generate(&bitcoind, 10);

    let _tx = client.get_tx(txid).unwrap();
}
