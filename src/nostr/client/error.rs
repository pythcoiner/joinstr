#[derive(Debug)]
pub enum Error {
    AlreadyConnected,
    NotConnected,
    Disconnected,
    Client(nostr_sdk::client::Error),
    EventBuilder(nostr_sdk::event::builder::Error),
    Signer(nostr_sdk::signer::Error),
    KeysMissing,
    NotNip04,
    DmEncryption,
    Serializing(crate::nostr::SerializeError),
}

impl From<crate::nostr::SerializeError> for Error {
    fn from(value: crate::nostr::SerializeError) -> Self {
        Self::Serializing(value)
    }
}

impl From<nostr_sdk::client::Error> for Error {
    fn from(value: nostr_sdk::client::Error) -> Self {
        Self::Client(value)
    }
}

impl From<nostr_sdk::event::builder::Error> for Error {
    fn from(value: nostr_sdk::event::builder::Error) -> Self {
        Self::EventBuilder(value)
    }
}

impl From<nostr_sdk::signer::Error> for Error {
    fn from(value: nostr_sdk::signer::Error) -> Self {
        Self::Signer(value)
    }
}
