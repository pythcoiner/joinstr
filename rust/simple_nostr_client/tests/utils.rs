use nostr::key::Keys;
use nostrd::NostrD;
use simple_nostr_client::WsClient;

pub struct Relay {
    nostrd: NostrD,
}

impl Relay {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        let nostrd = NostrD::new().unwrap();

        Relay { nostrd }
    }

    pub fn new_client(&self) -> WsClient {
        let keys = Keys::generate();
        self.new_client_with_keys(keys)
    }

    pub fn new_client_with_keys(&self, keys: Keys) -> WsClient {
        WsClient::new()
            .relay(self.nostrd.url())
            .keys(keys)
            .connect()
            .unwrap()
    }

    pub fn url(&self) -> String {
        self.nostrd.url()
    }
}

#[allow(dead_code)]
pub fn dump_nostr_log(relay: &mut Relay) {
    while let Ok(msg) = relay.nostrd.logs.try_recv() {
        log::info!("{msg}");
    }
}

#[allow(dead_code)]
pub fn clear_nostr_log(relay: &mut Relay) {
    while relay.nostrd.logs.try_recv().is_ok() {}
}
