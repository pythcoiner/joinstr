use std::fmt::Display;

#[derive(Debug)]
pub enum Error {
    TxAlreadyHasInput,
    SighashFail,
    InvalidSignature,
    InvalidTransaction,
    NoElectrumClient,
    CoinPathWithoutIndex,
    CoinPath,
    XPrivFromSeed,
    Derivation,
    Bip39(bip39::Error),
    Electrum(crate::electrum::Error),
}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::TxAlreadyHasInput => write!(f, "PSBT should only have outputs at this step"),
            Error::SighashFail => write!(f, "Sighash id not SIGHASH_ALL | SIGHASH_ANYONE_CAN_PAY"),
            Error::InvalidSignature => write!(f, "Signature processed is invalid"),
            Error::InvalidTransaction => write!(f, "Fail to create PSBT from unsigned transaction"),
            Error::NoElectrumClient => write!(f, "There is no electrum client provided"),
            Error::CoinPathWithoutIndex => write!(f, "Invalid CoinPath provided: index is missing"),
            Error::CoinPath => write!(f, "Wrong CoinPath"),
            Error::Electrum(e) => write!(f, "{}", e),
            Error::Bip39(e) => write!(f, "{}", e),
            Error::XPrivFromSeed => write!(f, "Fail to generate XPriv from seed"),
            Error::Derivation => write!(f, "Derivation fails"),
        }
    }
}

impl From<crate::electrum::Error> for Error {
    fn from(value: crate::electrum::Error) -> Self {
        Error::Electrum(value)
    }
}

impl From<bip39::Error> for Error {
    fn from(value: bip39::Error) -> Self {
        Error::Bip39(value)
    }
}
