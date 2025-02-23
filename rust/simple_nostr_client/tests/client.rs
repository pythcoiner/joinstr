use std::{sync::Once, time::Duration};

use env_logger::Env;
use joinstr::nostr::{default_version, Pool, PoolPayload, PoolType};
use joinstr::{
    miniscript::bitcoin::{Amount, Network},
    nostr::{Fee, Timeline, Transport, Vpn},
};
use nostr::event::EventBuilder;
use nostr::key::PublicKey;
use utils::{clear_nostr_log, Relay};

use crate::utils::dump_nostr_log;

mod utils;

static INIT: Once = Once::new();

fn init_logger() {
    INIT.call_once(|| {
        let env = Env::new().filter_or("TEST_LOG", "debug");
        let _ = env_logger::Builder::from_env(env).is_test(true).try_init();
    });
}

#[test]
pub fn test_dm() {
    init_logger();

    let mut relay = Relay::new();
    let mut client_a = relay.new_client();
    let mut client_b = relay.new_client();
    clear_nostr_log(&mut relay);

    client_a.subscribe_dm().unwrap();

    std::thread::sleep(Duration::from_secs(1));
    dump_nostr_log(&mut relay);

    client_b.send_dm("test dm", &client_a.pubkey()).unwrap();
    std::thread::sleep(Duration::from_secs(3));
    dump_nostr_log(&mut relay);

    loop {
        match client_a.try_receive() {
            Ok(Some(event)) => {
                #[allow(deprecated)]
                {
                    log::info!("receive event: {}", event.content());
                }
                #[allow(deprecated)]
                if let "test dm" = event.content() {
                    return;
                }
            }
            Err(e) => log::error!("{:?}", e),
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
        }
    }
}

#[test]
fn test_pool_notif() {
    init_logger();

    let mut relay = Relay::new();
    let mut client_a = relay.new_client();
    let mut client_b = relay.new_client();
    clear_nostr_log(&mut relay);

    client_a.subscribe_pool(60).unwrap();

    std::thread::sleep(Duration::from_secs(3));

    let pool = Pool {
        versions: default_version(),
        id: "123".into(),
        pool_type: PoolType::Create,
        public_key: PublicKey::parse(
            "0000000000000000000000000000000000000000000000000000000000000001",
        )
        .unwrap(),
        network: Network::Regtest,
        payload: Some(PoolPayload {
            denomination: Amount::from_btc(0.1).unwrap(),
            peers: 5,
            timeout: Timeline::Simple(12345),
            relays: Vec::new(),
            fee: Fee::Fixed(12),
            transport: Transport {
                vpn: Some(Vpn {
                    enable: false,
                    gateway: None,
                }),
                tor: None,
            },
        }),
    };

    let event = EventBuilder::try_from(pool).unwrap();
    client_b.post_event(event).unwrap();

    std::thread::sleep(Duration::from_secs(3));
    dump_nostr_log(&mut relay);

    let mut counter = 0usize;
    loop {
        if let Some(event) = client_a.try_receive().unwrap() {
            let _pool_notif: Pool = event.try_into().unwrap();
            break;
        }
        std::thread::sleep(Duration::from_secs(3));
        counter += 1;
        assert!(counter < 10);
    }
}
