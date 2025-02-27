use cpp_joinstr::{Coin, ListCoinsResult, Network, QString, QUrl};
use joinstr::{interface, miniscript::bitcoin};

#[cxx_qt::bridge]
pub mod cpp_joinstr {

    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;

        include!("cxx-qt-lib/qurl.h");
        type QUrl = cxx_qt_lib::QUrl;
    }

    extern "Rust" {

        fn list_coins(
            mnemonics: QString,
            electrum_address: QUrl,
            start_index: u32,
            stop_index: u32,
            network: Network,
        ) -> ListCoinsResult;
    }

    struct Coin {
        outpoint: QString,
        value: u64,
    }

    struct ListCoinsResult {
        coins: Vec<Coin>,
        error: String,
    }

    enum Network {
        /// Mainnet Bitcoin.
        Bitcoin,
        /// Bitcoin's testnet network.
        Testnet,
        /// Bitcoin's signet network.
        Signet,
        /// Bitcoin's regtest network.
        Regtest,
    }
}

impl From<Network> for bitcoin::Network {
    fn from(value: Network) -> Self {
        match value {
            Network::Bitcoin => Self::Bitcoin,
            Network::Testnet => Self::Testnet,
            Network::Signet => Self::Signet,
            Network::Regtest => Self::Regtest,
            _ => unreachable!(),
        }
    }
}

fn list_coins(
    mnemonics: QString,
    electrum_address: QUrl,
    start_index: u32,
    stop_index: u32,
    network: Network,
) -> ListCoinsResult {
    let electrum_port = electrum_address.port_or(-1);
    if electrum_port == -1 {
        return ListCoinsResult {
            coins: Vec::new(),
            error: "electrum_address.port must be specified!".to_string(),
        };
    }

    let res = interface::list_coins(
        mnemonics.to_string(),
        electrum_address.to_string(),
        electrum_port as u16,
        (start_index, stop_index),
        network.into(),
    );

    let mut result = ListCoinsResult {
        coins: Vec::new(),
        error: String::new(),
    };

    match res {
        Ok(r) => {
            result.coins = r
                .into_iter()
                .map(|c| Coin {
                    outpoint: c.outpoint.to_string().into(),
                    value: c.txout.value.to_sat(),
                })
                .collect()
        }
        Err(e) => result.error = format!("{:?}", e),
    }

    result
}
