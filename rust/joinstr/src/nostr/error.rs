use simple_nostr_client::nostr;

#[derive(Debug)]
pub enum Error {
    AlreadyConnected,
    NotConnected,
    Disconnected,
    EventBuilder(nostr::event::builder::Error),
    KeysMissing,
    NotNip04,
    DmEncryption,
    Serializing(crate::nostr::SerializeError),
    SyncClient(simple_nostr_client::Error),
    SyncClientBuilderMissing,
    #[cfg(feature = "async")]
    Signer(nostr_sdk::signer::Error),
    #[cfg(feature = "async")]
    AsyncClient(nostr_sdk::client::Error),
}

impl From<simple_nostr_client::Error> for Error {
    fn from(value: simple_nostr_client::Error) -> Self {
        Self::SyncClient(value)
    }
}

impl From<crate::nostr::SerializeError> for Error {
    fn from(value: crate::nostr::SerializeError) -> Self {
        Self::Serializing(value)
    }
}

impl From<nostr::event::builder::Error> for Error {
    fn from(value: nostr::event::builder::Error) -> Self {
        Self::EventBuilder(value)
    }
}

#[cfg(feature = "async")]
impl From<nostr_sdk::client::Error> for Error {
    fn from(value: nostr_sdk::client::Error) -> Self {
        Self::AsyncClient(value)
    }
}

#[cfg(feature = "async")]
impl From<nostr_sdk::signer::Error> for Error {
    fn from(value: nostr_sdk::signer::Error) -> Self {
        Self::Signer(value)
    }
}
