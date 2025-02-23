#[derive(Debug)]
pub enum Error {
    Nostr(crate::nostr::error::Error),
    Event(crate::nostr::EventError),
    Coinjoin(crate::coinjoin::Error),
    Electrum(crate::electrum::Error),
    PoolAlreadyCreated,
    PoolAlreadyExists,
    PoolNotExists,
    WrongDenomination,
    ParamMissing,
    DenominationAlreadySet,
    PeersAlreadySet,
    Min2Peers,
    TimeoutAlreadySet,
    FeeAlreadySet,
    PeerRegistration,
    NotEnoughPeers(usize, usize),
    NotYetImplemented,
    PeerCountNotMatch(usize, usize),
    Timeout,
    CoinjoinMissing,
    MissingFinalTx,
    PoolConnectionTimeout,
    PeerAndPoolKeysNotMatch,
    PoolPayloadMissing,
    FeeProviderNotImplemented,
    TimelineNotImplemented,
    WrongAddressNetwork,
    OutputMissing,
    InputMissing,
    UnsignedTxNotExists,
    SigningFail(String),
    SignerMissing,
    PsbtToInput,
    DenominationMissing,
    PeerMissing,
    TimeoutMissing,
    RelaysMissing,
    FeeMissing,
    TimelineDuration,
    AlreadyHaveInput,
    AlreadyHaveOutput,
}

impl From<crate::coinjoin::Error> for Error {
    fn from(value: crate::coinjoin::Error) -> Self {
        Self::Coinjoin(value)
    }
}

impl From<crate::nostr::error::Error> for Error {
    fn from(value: crate::nostr::error::Error) -> Self {
        Self::Nostr(value)
    }
}

impl From<crate::nostr::EventError> for Error {
    fn from(value: crate::nostr::EventError) -> Self {
        Self::Event(value)
    }
}

impl From<crate::electrum::Error> for Error {
    fn from(value: crate::electrum::Error) -> Self {
        Self::Electrum(value)
    }
}
