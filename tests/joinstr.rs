pub mod utils;
use std::{sync::Once, time::Duration};

use crate::utils::{bootstrap_electrs, funded_wallet_with_bitcoind};
use electrsd::bitcoind::bitcoincore_rpc::RpcApi;
use miniscript::bitcoin::Network;
use rust_joinstr::{
    electrum::Client,
    signer::{CoinPath, WpkhHotSigner},
    utils::now,
};

use nostr_sdk::{Event, Keys, Kind};
use nostrd::NostrD;
use rust_joinstr::{joinstr::Joinstr, nostr::client::NostrClient};
use tokio::time::sleep;

static INIT: Once = Once::new();

pub fn setup_logger() {
    INIT.call_once(|| {
        env_logger::builder()
            // Ensures output is only printed in test mode
            .is_test(true)
            .filter_level(log::LevelFilter::Debug)
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

    pub async fn new_client(&self, name: &str) -> NostrClient {
        let keys = Keys::generate();
        self.new_client_with_keys(keys, name).await
    }

    pub async fn new_client_with_keys(&self, keys: Keys, name: &str) -> NostrClient {
        let mut client = NostrClient::new(name)
            .relay(self.nostrd.url())
            .unwrap()
            .keys(keys)
            .unwrap();
        client.connect_nostr().await.unwrap();
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

#[tokio::test]
async fn simple_dm() {
    let relay = Relay::new();
    let client_a = relay.new_client("client_a").await;
    let mut client_b = relay.new_client("client_b").await;
    let mut client_c = relay.new_client("client_c").await;

    client_a
        .send_dm(&client_b.get_keys().unwrap().public_key(), "ping".into())
        .await
        .unwrap();
    let e;
    loop {
        sleep(Duration::from_millis(100)).await;
        let event = client_b.receive_event().unwrap();
        if let Some(ev) = event {
            e = ev;
            break;
        }
    }
    let dm = client_b.decrypt_dm(e).unwrap();
    let Event { kind, content, .. } = dm;
    assert_eq!(kind, Kind::EncryptedDirectMessage);
    assert_eq!("ping".to_string(), content);

    // Client C should not receive DM sent to B
    let event = client_c.receive_event().unwrap();
    assert!(event.is_none());
}

#[tokio::test]
async fn simple_coinjoin() {
    let mut relay = Relay::new();
    let relays = vec![relay.url()];
    let keys = Keys::generate();
    let (url, port, _electrsd, bitcoind) = bootstrap_electrs();

    let mut pool_listener = NostrClient::new("pool_listener")
        .relays(&relays)
        .unwrap()
        .keys(Keys::generate())
        .unwrap();
    pool_listener.connect_nostr().await.unwrap();
    // subscribe to 2020 event up to 1 day back in time
    pool_listener.subscribe_pools(24 * 60 * 60).await.unwrap();

    // start a separate coordinator
    let mut coordinator = Joinstr::new_initiator(
        keys.clone(),
        &relays,
        (&url, port),
        Network::Regtest,
        "initiator",
    )
    .await
    .unwrap()
    .denomination(0.01)
    .unwrap()
    .fee(10)
    .unwrap()
    .simple_timeout(now() + 60)
    .unwrap()
    .min_peers(2)
    .unwrap();

    let _coordinator_handle = tokio::spawn(async move {
        coordinator
            .start_coinjoin(None, Option::<&WpkhHotSigner>::None)
            .await
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
        sleep(Duration::from_millis(300)).await;
        clear_nostr_log(&mut relay);
    }

    log::info!("Received pool notification.");

    let mut signer = funded_wallet_with_bitcoind(&[0.011, 0.011], &bitcoind);
    let client = Client::new(&url, port).unwrap();
    signer.set_client(client);

    sleep(Duration::from_secs(2)).await;

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
        &relays,
        &pool,
        coins[0].1.clone(),
        address_a,
        Network::Regtest,
        "peer_a",
    )
    .await
    .unwrap();

    let mut peer_b = Joinstr::new_peer(
        &relays,
        &pool,
        coins[1].1.clone(),
        address_b,
        Network::Regtest,
        "peer_b",
    )
    .await
    .unwrap();

    let signer_a = signer.clone();
    let pool_a = pool.clone();
    let _peer_a = tokio::spawn(async move {
        let _ = peer_a.start_coinjoin(Some(pool_a), Some(&signer_a)).await;
    });

    let _peer_b = tokio::spawn(async move {
        let _ = peer_b.start_coinjoin(Some(pool), Some(&signer)).await;
    });

    let (coordinator,) = tokio::join!(_coordinator_handle);
    let final_tx = coordinator.unwrap().unwrap();
    let _tx = bitcoind
        .client
        .get_raw_transaction(&final_tx.compute_txid(), None)
        .unwrap();
}
