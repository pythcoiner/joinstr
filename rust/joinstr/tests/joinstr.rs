pub mod utils;
use std::{
    sync::Once,
    thread::{self, sleep},
    time::Duration,
};

use crate::utils::{bootstrap_electrs, funded_wallet_with_bitcoind};
use electrsd::bitcoind::bitcoincore_rpc::RpcApi;
use joinstr::{
    electrum::Client,
    signer::{CoinPath, WpkhHotSigner},
    utils::now,
};
use miniscript::bitcoin::Network;

use joinstr::{joinstr::Joinstr, nostr::sync::NostrClient};
use nostrd::NostrD;
use simple_nostr_client::nostr::{Event, Keys, Kind};

static INIT: Once = Once::new();

pub fn setup_logger() {
    INIT.call_once(|| {
        env_logger::builder()
            // Ensures output is only printed in test mode
            .is_test(true)
            .filter_level(log::LevelFilter::Info)
            .init();
    });
}

pub struct Relay {
    nostrd: NostrD,
}

impl Relay {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        let nostrd = NostrD::new().unwrap();
        setup_logger();

        Relay { nostrd }
    }

    pub fn new_client(&self, name: &str) -> NostrClient {
        let keys = Keys::generate();
        self.new_client_with_keys(keys, name)
    }

    pub fn new_client_with_keys(&self, keys: Keys, name: &str) -> NostrClient {
        let mut client = NostrClient::new(name)
            .relay(self.nostrd.url())
            .unwrap()
            .keys(keys)
            .unwrap();
        client.connect_nostr().unwrap();
        client
    }

    pub fn url(&self) -> String {
        self.nostrd.url()
    }
}

#[allow(unused)]
fn dump_nostr_log(relay: &mut Relay) {
    while let Ok(msg) = relay.nostrd.logs.try_recv() {
        log::info!("{msg}");
    }
}

fn clear_nostr_log(relay: &mut Relay) {
    while relay.nostrd.logs.try_recv().is_ok() {}
}

#[test]
fn simple_dm() {
    let relay = Relay::new();
    let mut client_a = relay.new_client("client_a");
    let mut client_b = relay.new_client("client_b");
    let mut client_c = relay.new_client("client_c");

    client_a
        .send_dm(&client_b.get_keys().unwrap().public_key(), "ping".into())
        .unwrap();
    let e;
    loop {
        sleep(Duration::from_millis(100));
        let event = client_b.receive_event().unwrap();
        if let Some(ev) = event {
            e = ev;
            break;
        }
    }
    let Event { kind, content, .. } = e;
    assert_eq!(kind, Kind::EncryptedDirectMessage);
    assert_eq!("ping".to_string(), content);

    // Client C should not receive DM sent to B
    let event = client_c.receive_event().unwrap();
    assert!(event.is_none());
}

#[test]
fn simple_coinjoin() {
    let mut relay = Relay::new();
    let relays = relay.url();
    let keys = Keys::generate();
    let (url, port, _electrsd, bitcoind) = bootstrap_electrs();

    let mut pool_listener = NostrClient::new("pool_listener")
        .relay(relays.clone())
        .unwrap()
        .keys(Keys::generate())
        .unwrap();
    pool_listener.connect_nostr().unwrap();
    // subscribe to 2020 event up to 1 day back in time
    pool_listener.subscribe_pools(24 * 60 * 60).unwrap();

    // start a separate coordinator
    let mut coordinator = Joinstr::new_initiator(
        keys.clone(),
        relays.clone(),
        (&url, port),
        Network::Regtest,
        "initiator",
    )
    .unwrap()
    .denomination(0.01)
    .unwrap()
    .fee(10)
    .unwrap()
    .simple_timeout(now() + 60)
    .unwrap()
    .min_peers(2)
    .unwrap();

    let coordinator_handle = thread::spawn(move || {
        coordinator
            .start_coinjoin(None, Option::<&WpkhHotSigner>::None)
            .unwrap();
        coordinator.final_tx().cloned()
    });

    clear_nostr_log(&mut relay);

    // wait for the 2022 event to be broadcast
    let pool;
    loop {
        if let Some(notif) = pool_listener.receive_pool_notification().unwrap() {
            pool = notif;
            break;
        }
        sleep(Duration::from_millis(300));
        clear_nostr_log(&mut relay);
    }

    log::info!("Received pool notification.");

    let mut signer = funded_wallet_with_bitcoind(&[0.011, 0.011], &bitcoind);
    let client = Client::new(&url, port).unwrap();
    signer.set_client(client);

    sleep(Duration::from_secs(2));

    // fetch coins on electrum server
    let coin = signer
        .get_coins_at(CoinPath {
            depth: 0,
            index: Some(0),
        })
        .unwrap();
    assert_eq!(coin, 1);

    let coin = signer
        .get_coins_at(CoinPath {
            depth: 0,
            index: Some(1),
        })
        .unwrap();
    assert_eq!(coin, 1);

    // get list of fetched coins
    let coins = signer.list_coins();
    assert_eq!(coins.len(), 2);

    let address_a = signer
        .address_at(&CoinPath {
            depth: 0,
            index: Some(100),
        })
        .unwrap()
        .as_unchecked()
        .clone();
    let address_b = signer
        .address_at(&CoinPath {
            depth: 0,
            index: Some(101),
        })
        .unwrap()
        .as_unchecked()
        .clone();

    let mut peer_a = Joinstr::new_peer(
        relays.clone(),
        &pool,
        coins[0].1.clone(),
        address_a,
        Network::Regtest,
        "peer_a",
    )
    .unwrap();

    let mut peer_b = Joinstr::new_peer(
        relays.clone(),
        &pool,
        coins[1].1.clone(),
        address_b,
        Network::Regtest,
        "peer_b",
    )
    .unwrap();

    let signer_a = signer.clone();
    let pool_a = pool.clone();
    let _peer_a = thread::spawn(move || {
        let _ = peer_a.start_coinjoin(Some(pool_a), Some(&signer_a));
    });

    let _peer_b = thread::spawn(move || {
        let _ = peer_b.start_coinjoin(Some(pool), Some(&signer));
    });

    let final_tx = coordinator_handle.join().unwrap().unwrap();
    let _tx = bitcoind
        .client
        .get_raw_transaction(&final_tx.compute_txid(), None)
        .unwrap();
}
