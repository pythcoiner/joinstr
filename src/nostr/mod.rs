pub mod client;

use bip39::serde::{Deserialize, Serialize};
use bitcoin::Address;
use hex_conservative::DisplayHex;
use miniscript::bitcoin::{
    address::NetworkUnchecked,
    consensus::encode::{deserialize_hex, serialize_hex},
    Amount, Network, Psbt, Transaction, TxIn,
};
use nostr_sdk::{Event, EventBuilder, Kind};
use serde::Serializer;
use serde_json::{Map, Value};
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq)]
pub struct InputDataSigned {
    pub txin: TxIn,
    pub amount: Option<Amount>,
}

pub enum Error {
    NoInput,
    TooMuchInputs,
    WitnessMissing,
}

impl TryFrom<Psbt> for InputDataSigned {
    type Error = Error;

    fn try_from(value: Psbt) -> Result<Self, Self::Error> {
        match value.inputs.len() {
            0 => return Err(Error::NoInput),
            i if i > 1 => return Err(Error::TooMuchInputs),
            _ => {}
        }
        match value.unsigned_tx.input.len() {
            0 => return Err(Error::NoInput),
            i if i > 1 => return Err(Error::TooMuchInputs),
            _ => {}
        }

        let mut txin = value.unsigned_tx.input[0].to_owned();

        if txin.witness.is_empty() {
            if let Some(witness) = &value.inputs[0].final_script_witness {
                txin.witness = witness.clone();
            } else {
                return Err(Error::WitnessMissing);
            }
        }
        Ok(InputDataSigned { txin, amount: None })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Pool {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default = "default_version")]
    pub versions: Option<Vec<String>>,
    pub id: String,
    pub network: Network,
    #[serde(rename = "type")]
    pub pool_type: PoolType,
    pub public_key: nostr_sdk::PublicKey,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(flatten)]
    pub payload: Option<PoolPayload>,
}

#[derive(Debug)]
pub enum EventError {
    ContentError,
    WrongKind,
    Parsing(ParsingError),
}

impl From<ParsingError> for EventError {
    fn from(value: ParsingError) -> Self {
        EventError::Parsing(value)
    }
}

impl TryFrom<Pool> for EventBuilder {
    type Error = EventError;
    fn try_from(value: Pool) -> Result<Self, EventError> {
        if let Ok(content) = serde_json::to_string(&value) {
            Ok(EventBuilder::new(Kind::Custom(2022), content, Vec::new()))
        } else {
            Err(EventError::ContentError)
        }
    }
}

impl TryFrom<Event> for Pool {
    type Error = EventError;

    fn try_from(event: Event) -> Result<Self, Self::Error> {
        if event.kind == Kind::Custom(2022) {
            serde_json::from_str(&event.content).map_err(|e| EventError::Parsing(e.into()))
        } else {
            Err(EventError::WrongKind)
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PoolType {
    #[serde(alias = "new_pool")]
    Create,
    Update,
    Delete,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PoolPayload {
    pub denomination: Amount,
    pub peers: usize,
    pub timeout: Timeline,
    pub relays: Vec<String>,
    #[serde(rename = "fee_rate")]
    pub fee: Fee,
    pub transport: Transport,
}

pub fn default_version() -> Option<Vec<String>> {
    Some(vec!["0".into()])
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
#[serde(untagged)]
pub enum Timeline {
    Simple(u64),
    Fixed {
        /// The absolute timestamp the pool coordinator will wait until cancelation of the pool
        /// Coordinator must close the pool if the peer number not reach at this point in time.
        /// Coordinator will wait until this point in time before starting the coinjoin, in order
        /// to let more user register if possible
        start: u64,
        /// The max duration in seconds the coordinator will wait signed inputs registration before cancel the coinjoin.
        max_duration: u64,
    },
    Timeout {
        /// The absolute timestamp the pool coordinator will wait until cancelation of the pool
        /// Coordinator must start the coinjoin as soon as the min peer number is reached
        timeout: u64,
        /// The max duration in seconds the coordinator will wait signed inputs registration before cancel the coinjoin
        max_duration: u64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
#[serde(untagged)]
pub enum Fee {
    /// The min fee expected to consider a coinjoin tx broadcastable
    Fixed(u32),
    /// Using a fee provider mechanism:
    ///   - every input should have the denomination amount
    ///   - one input can have an amount superior to the denomination amount: it will be considered
    ///     as a fee payout to the provider
    ///   - if the input containing a fee payout is superior then expected fee, the fee provider
    ///     should add an ouput to receive the payout, this should be determined early in the
    ///     coinjoin (before the signing round start).
    ///   - if the participant inputs did not provide enough fee, the fee provider must add an
    ///     input to pay fees.
    Provider(Provider),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct Provider {
    pub address: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct Transport {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vpn: Option<Vpn>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tor: Option<Tor>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct Vpn {
    pub enable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gateway: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct Tor {
    pub enable: bool,
}

#[derive(Debug, PartialEq)]
pub enum PoolMessage {
    Input(InputDataSigned),
    Output(miniscript::bitcoin::Address<NetworkUnchecked>),
    Psbt(Psbt),
    Transaction(Transaction),
    Join(Option<nostr_sdk::PublicKey>),
    Credentials(Credentials),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Credentials {
    pub id: String,
    #[serde(serialize_with = "serialize_key")]
    pub key: nostr_sdk::SecretKey,
}
pub fn serialize_key<S>(key: &nostr_sdk::SecretKey, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let str = key.secret_bytes().to_lower_hex_string();
    serializer.serialize_str(&str)
}

#[derive(Debug)]
pub enum ParsingError {
    SerdeJson(serde_json::Error),
    Unknown,
    UnknownType(String),
    Input,
    Output,
    Psbt,
    Inputs,
    Outputs,
    Transaction,
    Join,
    NotAnObject,
    NotAnArray,
    MissingKey(String),
    WrongValue(String),
    Consensus,
    Credential,
    VersionNotSupported(String),
    VersionMissing,
}

impl From<serde_json::Error> for ParsingError {
    fn from(value: serde_json::Error) -> Self {
        ParsingError::SerdeJson(value)
    }
}

impl FromStr for PoolMessage {
    type Err = ParsingError;

    fn from_str(s: &str) -> Result<Self, ParsingError> {
        let json: Value = serde_json::from_str(s)?;
        if let Value::Object(map) = json {
            match map.get("version") {
                Some(Value::String(v)) => {
                    if v != "1" {
                        return Err(ParsingError::VersionNotSupported(v.into()));
                    }
                }
                _ => return Err(ParsingError::VersionMissing),
            }
            if let Some(Value::String(t)) = map.get("type") {
                return match t.as_str() {
                    "psbt" => {
                        if let Some(Value::String(psbt)) = map.get("psbt") {
                            let psbt: Psbt = serde_json::from_str(psbt)?;
                            Ok(Self::Psbt(psbt))
                        } else {
                            Err(ParsingError::Psbt)
                        }
                    }
                    "input" => {
                        if let Some(m) = map.get("input") {
                            let input = InputDataSigned::from_value(m.clone())?;
                            Ok(Self::Input(input))
                        } else {
                            Err(ParsingError::Input)
                        }
                    }
                    "output" => {
                        if let Some(Value::String(addr)) = map.get("address") {
                            let addr: miniscript::bitcoin::Address<NetworkUnchecked> =
                                Address::from_str(addr).map_err(|_| ParsingError::Output)?;
                            Ok(Self::Output(addr))
                        } else {
                            Err(ParsingError::Output)
                        }
                    }
                    "transaction" => {
                        if let Some(Value::String(s)) = map.get("transaction") {
                            let tx: Result<Transaction, _> = deserialize_hex(s);
                            return if let Ok(tx) = tx {
                                Ok(Self::Transaction(tx))
                            } else {
                                Err(ParsingError::Transaction)
                            };
                        } else {
                            Err(ParsingError::Transaction)
                        }
                    }
                    "join_pool" => {
                        if let Some(value) = map.get("npub") {
                            let npub: nostr_sdk::PublicKey = serde_json::from_value(value.clone())?;
                            Ok(Self::Join(Some(npub)))
                        } else {
                            Ok(Self::Join(None))
                        }
                    }
                    "credentials" => {
                        if let Some(value) = map.get("credentials") {
                            let cred: Credentials = serde_json::from_value(value.clone())?;
                            Ok(Self::Credentials(cred))
                        } else {
                            Err(ParsingError::Credential)
                        }
                    }
                    t => {
                        return Err(ParsingError::UnknownType(t.into()));
                    }
                };
            };
        }
        Err(ParsingError::Unknown)
    }
}

#[derive(Debug)]
pub enum SerializeError {
    Transaction,
    Inputs,
    Outputs,
    SerdeJson(serde_json::Error),
}

impl From<serde_json::Error> for SerializeError {
    fn from(value: serde_json::Error) -> Self {
        Self::SerdeJson(value)
    }
}

impl InputDataSigned {
    pub fn to_json(&self) -> Value {
        let mut map = Map::new();
        map.insert("txin".into(), Value::String(serialize_hex(&self.txin)));
        // `serialize_hex()` does not serialize the witness so a separate field is used
        let witness = &self.txin.witness;
        map.insert("witness".into(), Value::String(serialize_hex(witness)));
        if let Some(amount) = self.amount {
            map.insert("amount".into(), amount.to_sat().into());
        }
        map.into()
    }

    pub fn to_string(&self) -> Result<String, SerializeError> {
        let json = self.to_json();
        Ok(serde_json::to_string(&json)?)
    }

    pub fn to_string_pretty(&self) -> Result<String, SerializeError> {
        let json = self.to_json();
        Ok(serde_json::to_string_pretty(&json)?)
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<Self, ParsingError> {
        let value: Value = serde_json::from_str(value)?;
        Self::from_value(value)
    }

    pub fn from_value(value: Value) -> Result<Self, ParsingError> {
        if let Value::Object(map) = value {
            let txin = map
                .get("txin")
                .ok_or(ParsingError::MissingKey("txin".into()))?;
            let mut txin: TxIn = if let Value::String(str) = txin {
                deserialize_hex(str).map_err(|_| ParsingError::Consensus)?
            } else {
                return Err(ParsingError::WrongValue("txin".into()));
            };

            // `serialize_hex()` does not serialize the witness so a separate field is used
            let witness = map
                .get("witness")
                .ok_or(ParsingError::MissingKey("witness".into()))?;
            let witness = if let Value::String(str) = witness {
                deserialize_hex(str).map_err(|_| ParsingError::Consensus)?
            } else {
                return Err(ParsingError::WrongValue("witness".into()));
            };
            txin.witness = witness;

            let amount = map
                .get("amount")
                .ok_or(ParsingError::MissingKey("amount".into()))?;
            let amount: Option<Amount> = Some(serde_json::from_value(amount.clone())?);
            Ok(Self { txin, amount })
        } else {
            Err(ParsingError::NotAnObject)
        }
    }
}

impl PoolMessage {
    pub fn to_json(&self) -> Result<Value, SerializeError> {
        let msg_type = match self {
            PoolMessage::Input(_) => "input",
            PoolMessage::Output(_) => "output",
            PoolMessage::Psbt(_) => "psbt",
            PoolMessage::Transaction(_) => "transaction",
            PoolMessage::Join(_) => "join_pool",
            PoolMessage::Credentials(_) => "credentials",
        };
        let mut map = Map::new();
        map.insert("version".into(), Value::String("1".into()));
        map.insert("type".into(), msg_type.into());
        match self {
            PoolMessage::Psbt(psbt) => {
                map.insert(msg_type.into(), serde_json::to_value(psbt)?);
            }
            PoolMessage::Transaction(tx) => {
                let raw = serialize_hex(tx);
                map.insert(msg_type.into(), Value::String(raw));
            }
            PoolMessage::Join(npub) => {
                if let Some(npub) = npub {
                    map.insert("npub".into(), serde_json::to_value(npub)?);
                }
            }
            PoolMessage::Input(input) => {
                map.insert(msg_type.into(), input.to_json());
            }
            PoolMessage::Output(addr) => {
                map.insert("address".into(), serde_json::to_value(addr)?);
            }
            PoolMessage::Credentials(cred) => {
                map.insert(msg_type.into(), serde_json::to_value(cred)?);
            }
        }
        Ok(map.into())
    }

    pub fn to_string(&self) -> Result<String, SerializeError> {
        let json = self.to_json()?;
        let str = serde_json::to_string(&json)?;
        Ok(str)
    }

    pub fn to_string_pretty(&self) -> Result<String, SerializeError> {
        let json = self.to_json()?;
        let str = serde_json::to_string_pretty(&json)?;
        Ok(str)
    }
}

#[cfg(test)]
pub mod tests {
    use nostr_sdk::{Keys, PublicKey};

    use super::*;
    const RAW_POOL: &str = r#"
            {
              "version": "1",
              "type": "create",
              "id": "123",
              "public_key": "0000000000000000000000000000000000000000000000000000000000000001",
              "network": "regtest",
              "denomination": 10000000,
              "peers": 5,
              "timeout": 12345,
              "relays": [],
              "fee_rate": 12,
              "transport": {
                "vpn": {
                  "enable": false
                }
              }
            }
        "#;
    #[test]
    fn pool() {
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

        let raw = RAW_POOL;

        let parsed: Pool = serde_json::from_str(raw).unwrap();
        assert_eq!(pool, parsed);
    }

    #[test]
    fn input_data_signed() {
        let raw = r#"
            {
              "txin": "4f8176ffbca02baba974a4458eae799a87afa8a00317565827f035a8d45556ba0000000000fdffffff",
              "witness": "0247304402202be1d200c2c917c6bda981dd56b55a272f06af9aca9af4f9c8a23d4d0429bc420220623b571410104edc7773ab5cf71f3e10f814028aedef133591c1dab74eefc51f812103b1ea5528a8279cf184e76464ba5ed0a80cc6ca7c47899478fb7e4c9411404877",
              "amount": 1000000
            }
        "#;
        let ids = InputDataSigned::from_str(raw).unwrap();
        let serialized = ids.to_string().unwrap();
        let roundtrip = InputDataSigned::from_str(&serialized).unwrap();
        assert_eq!(ids, roundtrip);
    }

    #[test]
    fn input() {
        let raw = r#"
            {
              "version": "1",
              "type": "input",
              "input": 
                {
                  "txin": "4f8176ffbca02baba974a4458eae799a87afa8a00317565827f035a8d45556ba0000000000fdffffff",
                  "witness": "0247304402202be1d200c2c917c6bda981dd56b55a272f06af9aca9af4f9c8a23d4d0429bc420220623b571410104edc7773ab5cf71f3e10f814028aedef133591c1dab74eefc51f812103b1ea5528a8279cf184e76464ba5ed0a80cc6ca7c47899478fb7e4c9411404877",
                  "amount": 1000000
                }
            }
        "#;
        let msg = PoolMessage::from_str(raw).unwrap();
        assert!(matches!(msg, PoolMessage::Input(_)));
        let serialized = msg.to_string().unwrap();
        let roundtrip = PoolMessage::from_str(&serialized).unwrap();
        assert_eq!(msg, roundtrip);
    }

    #[test]
    fn output() {
        let raw = r#"
            {
                "version": "1",
                "type": "output",
                "address": "bc1q4smd35jchznp0u442zhyv5yawf200ffet5kqc9"
            }
        "#;
        let msg = PoolMessage::from_str(raw).unwrap();
        assert!(matches!(msg, PoolMessage::Output(_)));
        let serialized = msg.to_string().unwrap();
        let roundtrip = PoolMessage::from_str(&serialized).unwrap();
        assert_eq!(msg, roundtrip);
    }

    #[test]
    fn join() {
        let raw = r#"
            {
              "version": "1",
                "type": "join_pool"
            }
        "#;
        let msg = PoolMessage::from_str(raw).unwrap();
        assert!(matches!(msg, PoolMessage::Join(None)));
        let serialized = msg.to_string().unwrap();
        let roundtrip = PoolMessage::from_str(&serialized).unwrap();
        assert_eq!(msg, roundtrip);
    }

    #[test]
    fn join_npub() {
        let raw = r#"
            {
              "version": "1",
                "type": "join_pool",
                "npub": "0000000000000000000000000000000000000000000000000000000000000001"
            }
        "#;
        let msg = PoolMessage::from_str(raw).unwrap();
        assert!(matches!(msg, PoolMessage::Join(Some(_))));
        let serialized = msg.to_string().unwrap();
        let roundtrip = PoolMessage::from_str(&serialized).unwrap();
        assert_eq!(msg, roundtrip);
    }

    #[test]
    fn transaction() {
        let raw = r#"
            {
                "version": "1",
                "type": "transaction",
                "transaction": "020000000001050121a999ecdcd0f288d300e28719fcf859dae2b3b644292b5f2d2aecbb2e3de20100000000fdffffff16ccb309f4f9975b983b4bba70b4ab71d1721644f8204f816e549e94016a45d90100000000fdffffff9147ae6adeee48ad24f927e54779be2d1036c87fe04777e847e258a8a709d1d40000000000fdffffff423ce31bbdab2b76296ad846742539957951af6ab1587cf9e07eeb8aba0c51dc0100000000fdffffff5188353f0c80629d4efbf8753299ed09587c2d070d10b51d43127eac1dc8ef0c0100000000fdffffff0580969800000000001600140300036b09933e01bedbeae63850f29abc49164a8096980000000000160014185cede11f852fd6ca22d6b3f2383602f4639adb80969800000000001600142354a0caffe52b6521052e439327d8a41311e89780969800000000001600149ab1671aeb5cf5a2646044dac08a6aac1c3b5ca28096980000000000160014e60577e0f4a1a16837073810aad2d61d6810393d02473044022054a76a3f40098f18f202bebc4c373117b724e0de0ee44606a1570a9f123a648c022036b33c776a7bee07d936219333f2110a4bf1e966be52bca2cf79c8b4d384198e8121033cf9267abc8d886ee38c1ba032603099c0fbc8dfadcfcb43ab66bd0467e4adb90247304402202d7573747931547cb4c8ef26883435d3692357c7c92248e2e1b4327517aa476d02207be4bcde6284773fef9ce3794665a579c23a8805cc31c68405c42c0aea9c3af881210273d5ceafd9f15aa4b39ed7f0ab7f2dc365e4ccfaf9d88dd741d1e238a060fb0a024730440220634fb1d0dd7ab4726921caf790c41d3329a9685e6317afa5110c5b2bc9e22caf02200a8e76ace3e1bbda10da8c982b99658fa65e099bdd6c9c0209145f252c5a3c4d81210292f420e0790da79e55d55f9ecbe03a2545bc09d5bff0bf3268eb7779fb580b5d024730440220452f0c4ae26f0910290f62bf2ada8052e07d5ee7ebfed054a85391691d68c0640220461453b516902a07fc3c8fd206883a0e4f329f20da327af53d8a8edc4e6fae95812102445efffa41d0cf45382ceb6b5b02a16ee1e61957a91813ced84a2091ead6495102473044022017bc5084bab4c6ce796edd9f04e65463f4d0a2cb08b97be356e673d2982dddee02205b965d73cd4ad1c8647d76d5e201d718c315b3dc3c5987c6bc447bf1cac2b0ab812102fed36ddfe86cc993964b114780d66ff9c73867b7eae75d18e67a8d0490d68f6900000000"
            }
        "#;
        let msg = PoolMessage::from_str(raw).unwrap();
        assert!(matches!(msg, PoolMessage::Transaction(_)));
        let serialized = msg.to_string().unwrap();
        let roundtrip = PoolMessage::from_str(&serialized).unwrap();
        assert_eq!(msg, roundtrip);
    }

    #[test]
    fn credential() {
        let raw = r#"
            {
                "version": "1",
                "type": "credentials",
                "credentials": {
                  "id": "1234",
                  "key": "0000000000000000000000000000000000000000000000000000000000000001"
                }
            }
        "#;
        let msg = PoolMessage::from_str(raw).unwrap();
        assert!(matches!(msg, PoolMessage::Credentials(_)));
        let serialized = msg.to_string().unwrap();
        let roundtrip = PoolMessage::from_str(&serialized).unwrap();
        assert_eq!(msg, roundtrip);
    }

    #[test]
    fn pool_event() {
        let raw = RAW_POOL;
        let pool: Pool = serde_json::from_str(raw).unwrap();
        let keys = Keys::generate();
        let builder: EventBuilder = pool.clone().try_into().unwrap();
        let event = builder.to_event(&keys).unwrap();
        let roundtrip: Pool = event.try_into().unwrap();
        assert_eq!(pool, roundtrip);
    }
}
