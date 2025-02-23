use super::types::ScriptHash;
use miniscript::bitcoin::Txid;
use miniscript::serde::{ser::SerializeSeq, Deserialize, Serialize, Serializer};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(untagged)]
pub enum VersionKind {
    Single(String),
    MinMax(String, String),
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(untagged)]
pub enum TxGetArgs {
    Txid((Txid,)),
    TxidVerbose(Txid, bool),
}

impl From<&TxGetArgs> for (Txid, bool) {
    fn from(args: &TxGetArgs) -> Self {
        match args {
            TxGetArgs::Txid((txid,)) => (*txid, false),
            TxGetArgs::TxidVerbose(txid, verbose) => (*txid, *verbose),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(untagged)]
pub enum Params {
    #[serde(serialize_with = "default")]
    None,
    // NOTE: electrs does not support `cp_height` even if
    // it's in the 1.4 version spec. ...
    // https://electrumx.readthedocs.io/en/latest/protocol-methods.html#blockchain-block-header
    // BlockHeader((usize /* height*/, usize /* cp_height */)),
    BlockHeader((usize /* height*/,)),
    // NOTE: idem
    BlockHeaders(
        (
            usize, /* start */
            usize, /* count */
                   // usize, /* cp_height */
        ),
    ),
    TransactionBroadcast((String,)),
    EstimateFee((u16,)),
    ScriptHashGetBalance((ScriptHash,)),
    ScriptHashGetHistory((ScriptHash,)),
    ScriptHashGetMempool((ScriptHash,)),
    ScriptHashListUnspent((ScriptHash,)),
    ScriptHashSubscribe((ScriptHash,)),
    ScriptHashUnsubscribe((ScriptHash,)),
    TransactionGet(TxGetArgs),
    TransactionGetMerkle((Txid, usize)),
    TransactionFromPosition(
        (
            usize, /*height*/
            usize, /*tx_pos*/
            bool,  /*merkle*/
        ),
    ),
    Version((String, VersionKind)),
}

fn default<S>(serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    // Serialize unit type as an empty array "[]"
    let sequence = serializer.serialize_seq(Some(0))?;
    sequence.end()
}

#[cfg(test)]
mod tests {
    use super::*;
    use miniscript::bitcoin::{hex::FromHex, OutPoint, Script};
    use std::str::FromStr;

    macro_rules! json {
        ($value:expr, $str:expr) => {{
            use serde_json::to_string;

            let json_str = to_string(&$value).unwrap();
            // let json_str = json_str.replace('"', "");

            assert_eq!(
                json_str, $str,
                "Debug and JSON representations do not match"
            );
        }};
    }

    #[test]
    fn params() {
        assert_eq!(serde_json::to_string(&Params::None).unwrap(), "[]");
        assert_eq!(
            serde_json::to_string(&Params::BlockHeader((0,))).unwrap(),
            "[0]"
        );
    }

    #[test]
    fn tx_get_args() {
        let outpoint = OutPoint::from_str(
            "5df6e0e2761359d30a8275058e299fcc0381534545f55cf43e41983f5d4c9456:42",
        )
        .unwrap();
        let arg1 = TxGetArgs::Txid((outpoint.txid,));

        let arg2 = TxGetArgs::TxidVerbose(outpoint.txid, true);

        assert_eq!(
            arg1,
            serde_json::from_str(
                r#"["5df6e0e2761359d30a8275058e299fcc0381534545f55cf43e41983f5d4c9456"]"#
            )
            .unwrap()
        );
        assert_eq!(
            arg2,
            serde_json::from_str(
                r#"["5df6e0e2761359d30a8275058e299fcc0381534545f55cf43e41983f5d4c9456",true]"#
            )
            .unwrap()
        );
    }

    #[test]
    fn from_tx_get_arg() {
        let outpoint = OutPoint::from_str(
            "5df6e0e2761359d30a8275058e299fcc0381534545f55cf43e41983f5d4c9456:42",
        )
        .unwrap();
        let arg1 = TxGetArgs::Txid((outpoint.txid,));

        let arg2 = TxGetArgs::TxidVerbose(outpoint.txid, true);

        let (txid, verbose): (Txid, bool) = (&arg1).into();
        assert_eq!(txid, outpoint.txid);
        assert!(!verbose);

        let (txid, verbose): (Txid, bool) = (&arg2).into();
        assert_eq!(txid, outpoint.txid);
        assert!(verbose);
    }

    #[test]
    fn version_kind() {
        let version1 = VersionKind::Single("1.4".into());
        let version2 = VersionKind::MinMax("1.1".into(), "1.4".into());

        assert_eq!(version1, serde_json::from_str(r#""1.4""#).unwrap());
        assert_eq!(version2, serde_json::from_str(r#"["1.1","1.4"]"#).unwrap());
    }

    #[test]
    fn params_() {
        json!(Params::None, "[]");
        json!(Params::BlockHeader((12,)), "[12]");
        json!(Params::BlockHeaders((12, 34)), "[12,34]");
        json!(Params::TransactionBroadcast(("toto".into(),)), "[\"toto\"]");
        json!(Params::EstimateFee((2,)), "[2]");

        let raw_script = Vec::from_hex("0014992f8cc4f6d284acac5f603e233592b566c04b2a").unwrap();
        let script = Script::from_bytes(raw_script.as_slice());
        let sh = ScriptHash::new(script);
        json!(
            Params::ScriptHashGetBalance((sh,)),
            "[\"8b2154ad6733677e53c2b9fd12d527bf292ace4df41281755ce1ecabe456fce5\"]"
        );
        json!(
            Params::ScriptHashGetHistory((sh,)),
            "[\"8b2154ad6733677e53c2b9fd12d527bf292ace4df41281755ce1ecabe456fce5\"]"
        );
        json!(
            Params::ScriptHashGetMempool((sh,)),
            "[\"8b2154ad6733677e53c2b9fd12d527bf292ace4df41281755ce1ecabe456fce5\"]"
        );
        json!(
            Params::ScriptHashListUnspent((sh,)),
            "[\"8b2154ad6733677e53c2b9fd12d527bf292ace4df41281755ce1ecabe456fce5\"]"
        );
        json!(
            Params::ScriptHashSubscribe((sh,)),
            "[\"8b2154ad6733677e53c2b9fd12d527bf292ace4df41281755ce1ecabe456fce5\"]"
        );
        json!(
            Params::ScriptHashUnsubscribe((sh,)),
            "[\"8b2154ad6733677e53c2b9fd12d527bf292ace4df41281755ce1ecabe456fce5\"]"
        );

        let outpoint = OutPoint::from_str(
            "5df6e0e2761359d30a8275058e299fcc0381534545f55cf43e41983f5d4c9456:42",
        )
        .unwrap();
        let arg1 = TxGetArgs::Txid((outpoint.txid,));

        let arg2 = TxGetArgs::TxidVerbose(outpoint.txid, true);

        json!(
            Params::TransactionGet(arg1),
            "[\"5df6e0e2761359d30a8275058e299fcc0381534545f55cf43e41983f5d4c9456\"]"
        );
        json!(
            Params::TransactionGet(arg2),
            "[\"5df6e0e2761359d30a8275058e299fcc0381534545f55cf43e41983f5d4c9456\",true]"
        );
        json!(
            Params::TransactionGetMerkle((outpoint.txid, 3)),
            "[\"5df6e0e2761359d30a8275058e299fcc0381534545f55cf43e41983f5d4c9456\",3]"
        );
        json!(
            Params::TransactionFromPosition((1, 2, false)),
            "[1,2,false]"
        );
        json!(
            Params::Version(("last".into(), VersionKind::Single("1.4.into())".into()))),
            "[\"last\",\"1.4.into())\"]"
        );
    }
}
