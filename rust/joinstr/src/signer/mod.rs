mod error;
pub use error::Error;
use serde::{Deserialize, Serialize};

use crate::{electrum::Client, nostr::InputDataSigned};
use bip39::Mnemonic;
use miniscript::{
    bitcoin::{
        bip32::{self, ChildNumber, DerivationPath, Fingerprint, Xpriv, Xpub},
        ecdsa,
        psbt::{self, PsbtSighashType},
        secp256k1::{self, All},
        sighash, Address, CompressedPublicKey, EcdsaSighashType, Network, OutPoint, PrivateKey,
        Psbt, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness,
    },
    descriptor::{DerivPaths, DescriptorMultiXKey, Wildcard},
    Descriptor, DescriptorPublicKey,
};
use std::{collections::HashMap, fmt::Debug, str::FromStr};

const MAX_DERIV: u32 = 2u32.pow(31) - 1;

pub trait JoinstrSigner {
    fn sign_input(&self, tx: &Transaction, input_data: Coin) -> Result<InputDataSigned, String>;
}

// S: JoinstrSigner + Sync + Clone + Send + 'static,
#[derive(Clone)]
pub struct WpkhHotSigner {
    #[allow(unused)]
    key: PrivateKey,
    master_xpriv: Xpriv,
    fingerprint: bip32::Fingerprint,
    secp: secp256k1::Secp256k1<All>,
    mnemonic: Option<Mnemonic>,
    secret_key: DescriptorMultiXKey<Xpriv>,
    network: Network,
    coins: HashMap<CoinPath, Vec<Coin>>,
    client: Option<Client>,
}

impl Debug for WpkhHotSigner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("P2WSHHotSigner").finish()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Coin {
    pub txout: TxOut,
    pub outpoint: OutPoint,
    pub sequence: Sequence,
    pub coin_path: CoinPath,
}

#[derive(Debug, Eq, Hash, PartialEq, Clone, Copy, Serialize, Deserialize)]
pub struct CoinPath {
    pub depth: u32,
    pub index: Option<u32>,
}

impl CoinPath {
    pub fn new(depth: u32, index: u32) -> Self {
        CoinPath {
            depth,
            index: Some(index),
        }
    }
}

pub fn descriptor(
    xpub: &Xpub,
    fg: &Fingerprint,
    multipath: u32,
) -> Descriptor<DescriptorPublicKey> {
    let descr_str = format!("wpkh([{}/84'/0'/0']{}/{}/*)", fg, xpub, multipath);

    Descriptor::<DescriptorPublicKey>::from_str(&descr_str).expect("descriptor")
}

impl WpkhHotSigner {
    /// Create a new [`WpkhHotSigner`] instance from the Xpriv key.
    ///
    /// # Arguments
    /// * `network` - The bitcoin network (bitcoin/testnet/signet/regtest)
    /// * `xpriv` - The private key the signer will use
    pub fn new_from_xpriv(network: Network, xpriv: Xpriv) -> Self {
        let secp = secp256k1::Secp256k1::new();
        let fingerprint = xpriv.fingerprint(&secp);

        let secret_key = DescriptorMultiXKey {
            origin: Some((
                fingerprint,
                DerivationPath::from_str("m/84'/0'/0'").expect("hardcoded"),
            )),
            xkey: xpriv,
            derivation_paths: DerivPaths::new(vec![
                vec![ChildNumber::from_normal_idx(0).expect("hardcoded")].into(),
                vec![ChildNumber::from_normal_idx(1).expect("hardcoded")].into(),
            ])
            .expect("hardcoded"),
            wildcard: Wildcard::Unhardened,
        };

        WpkhHotSigner {
            key: xpriv.to_priv(),
            master_xpriv: xpriv,
            fingerprint,
            secp,
            mnemonic: None,
            network,
            secret_key,
            coins: HashMap::new(),
            client: None,
        }
    }

    /// Create a new [`WpkhHotSigner`] instance from a mnemonic phrase.
    /// The mnemonic is stored in [`WpkhHotSigner::mnemonic`] field.
    ///
    /// # Arguments
    /// * `network` - The bitcoin network (bitcoin/testnet/signet/regtest)
    /// * `xpriv` - The private key the signer will use
    pub fn new_from_mnemonics(network: Network, mnemonic: &str) -> Result<Self, Error> {
        let mnemonic = Mnemonic::from_str(mnemonic)?;
        let seed = mnemonic.to_seed("");
        let key = bip32::Xpriv::new_master(network, &seed).map_err(|_| Error::XPrivFromSeed)?;
        Ok(Self::new_from_xpriv(network, key))
    }

    /// Generate a new signer and it's private key.
    /// The mnemonic is stored in [`WpkhHotSigner::mnemonic`] field.
    ///
    /// Note: generating a private key by this way is not safe enough
    ///   to use on mainnet, so we decide to forbid usage of this method on mainnet.
    ///   This method will panic if `network` have [`Network::Bitcoin`] value.
    pub fn new(network: Network) -> Result<Self, Error> {
        // Should not be used on mainnet
        assert_ne!(network, Network::Bitcoin);
        let mnemonic = Mnemonic::generate(12).expect("12 words must not fail");
        let mut signer = Self::new_from_mnemonics(network, &mnemonic.to_string())?;
        signer.mnemonic = Some(mnemonic);
        Ok(signer)
    }

    /// Set the electrum client to be used by the signer.
    /// Note: the signer need a bitcoin backend client in coinjoin context
    ///   in order to verify amounts of the inputs of a transaction.
    pub fn client(mut self, client: Client) -> Self {
        self.set_client(client);
        self
    }

    /// Set the electrum client to be used by the signer.
    /// Note: the signer need a bitcoin backend client in coinjoin context
    ///   in order to verify amounts of the inputs of a transaction.
    pub fn set_client(&mut self, client: Client) {
        if self.client.is_none() {
            self.client = Some(client);
        }
    }

    /// Remove the inner electrum client.
    pub fn drop_client(&mut self) {
        self.client = None;
    }

    /// Process the address for the given CoinPath.
    ///
    /// # Errors
    ///
    /// This function will return an error if the [`CoinPath::index`] is None
    pub fn address_at(&self, coin_path: &CoinPath) -> Result<Address, Error> {
        if let Some(index) = coin_path.index {
            let fingerprint = self.master_xpriv.fingerprint(self.secp());
            let xpub = Xpub::from_priv(self.secp(), &self.master_xpriv);
            let descriptor = descriptor(&xpub, &fingerprint, coin_path.depth);
            let definite = descriptor.at_derivation_index(index).expect("wildcard");
            Ok(definite.address(self.network).expect("wpkh"))
        } else {
            Err(Error::CoinPathWithoutIndex)
        }
    }

    /// Process the spk for the given CoinPath.
    ///
    /// # Errors
    ///
    /// This function will return an error if the [`CoinPath::index`] is None
    pub fn spk_at(&self, coin_path: &CoinPath) -> Result<ScriptBuf, Error> {
        Ok(self.address_at(coin_path)?.script_pubkey())
    }

    /// Use the inner electrum client to get coins that have been paid
    ///   to the given [`CoinPath`]. coins are automatically added to
    ///   [`WpkhHotSigner::coins`] and the functions return the number
    ///   of coins added.
    /// Note: this method is used to avoid address reuse.
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    ///   - an electrum request fails.
    ///   - there is not electrum client
    pub fn get_coins_at(&mut self, coin_path: CoinPath) -> Result<usize, Error> {
        let spk = self.spk_at(&coin_path)?;
        if let Some(client) = self.client.as_mut() {
            let (coins, _txs) = client.get_coins_at(&spk)?;
            let mut count = 0;
            for (txout, outpoint) in coins {
                // TODO: should we enable RBF?
                let sequence = Sequence::ENABLE_RBF_NO_LOCKTIME;
                let input_data = Coin {
                    txout,
                    outpoint,
                    sequence,
                    coin_path,
                };
                if let Some(coins) = self.coins.get_mut(&coin_path) {
                    coins.push(input_data);
                } else {
                    self.coins.insert(coin_path, vec![input_data]);
                }
                count += 1;
            }

            Ok(count)
        } else {
            Err(Error::NoElectrumClient)
        }
    }

    /// Returns a list of coins copied from [`WpkhHotSigner::coins`]
    ///
    /// Note: [`WpkhHotSigner::get_coins_at()`] should be call before in order to
    ///   fill [`WpkhHotSigner::coins`].
    pub fn list_coins(&self) -> Vec<(CoinPath, Coin)> {
        let mut out = Vec::new();
        let keys: Vec<_> = self.coins.keys().cloned().collect();
        for k in keys {
            if let Some(coins) = self.coins.get(&k) {
                for c in coins {
                    out.push((k, c.clone()));
                }
            } else {
                unreachable!()
            }
        }
        out
    }

    /// Sign the transaction w/ the given [`Coin`] as input. Returns the signed input
    ///   only as a [`InputDataSigned`].
    ///
    /// # Arguments
    /// * `tx` - the [`Transaction`] to be signed. Note: the transaction should not have any
    ///   inputs.
    /// * `input_data` - the [`Coin`] to be added as input.
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    ///   - a PSBT fail to be generated from the transaction
    ///   - the PSBT [`Psbt::inputs`] is not empty
    ///   - fail to process the spk for the given input
    ///   - fail to hash the transaction
    ///   - the signature generated is invalid
    pub fn sign(&self, tx: &Transaction, input_data: Coin) -> Result<InputDataSigned, Error> {
        let mut psbt = match Psbt::from_unsigned_tx(tx.clone()) {
            Ok(psbt) => psbt,
            Err(_) => return Err(Error::InvalidTransaction),
        };

        // the PSBT should have only outputs
        if !psbt.inputs.is_empty() {
            return Err(Error::TxAlreadyHasInput);
        }

        let spk = self
            .spk_at(&input_data.coin_path)
            .map_err(|_| Error::CoinPath)?;

        if input_data.txout.script_pubkey != spk {
            return Err(Error::CoinPath);
        }

        let input = psbt::Input {
            witness_utxo: Some(input_data.txout.clone()),
            // SIGHASH_ALL | SIGHASH_ANYONECANPAY
            sighash_type: Some(PsbtSighashType::from_u32(0x81)),
            ..Default::default()
        };
        psbt.inputs.push(input);

        let mut txin = TxIn {
            previous_output: input_data.outpoint,
            sequence: input_data.sequence,
            ..Default::default()
        };
        psbt.unsigned_tx.input.push(txin.clone());

        let mut cache = sighash::SighashCache::new(psbt.unsigned_tx.clone());
        // FIXME: process sighash w/o psbt helper?
        let (msg, sighash_type) = psbt
            .sighash_ecdsa(0, &mut cache)
            .map_err(|_| Error::SighashFail)?;
        if sighash_type != EcdsaSighashType::AllPlusAnyoneCanPay {
            return Err(Error::SighashFail);
        }

        let deriv = DerivationPath::from_str(&format!(
            "m/{}/{}",
            input_data.coin_path.depth,
            input_data
                .coin_path
                .index
                .expect("coinpath already checked")
        ))
        .expect("hardcoded");

        let signing_key = self
            .secret_key
            .xkey
            .derive_priv(self.secp(), &deriv)
            .expect("deriveable")
            .private_key;

        let pubkey = signing_key.public_key(self.secp());

        // check the keys matching utxo script_pubkey
        let comp = CompressedPublicKey(pubkey);
        let expected_spk = Address::p2wpkh(&comp, self.network).script_pubkey();
        // FIXME: we should error instead of panic here
        assert_eq!(expected_spk, input_data.txout.script_pubkey);

        let signature = self.secp.sign_ecdsa_low_r(&msg, &signing_key);

        if self.secp().verify_ecdsa(&msg, &signature, &pubkey).is_err() {
            return Err(Error::InvalidSignature);
        }
        let signature = ecdsa::Signature {
            signature,
            sighash_type: EcdsaSighashType::AllPlusAnyoneCanPay,
        };
        let wit = Witness::p2wpkh(&signature, &pubkey);
        txin.witness = wit;

        Ok(InputDataSigned {
            txin,
            amount: Some(input_data.txout.value),
        })
    }

    /// Returns the [`Fingerprint`] of this [`WpkhHotSigner`].
    fn fingerprint(&self) -> Fingerprint {
        self.fingerprint
    }

    /// Return the secp context of this signer
    fn secp(&self) -> &secp256k1::Secp256k1<All> {
        &self.secp
    }

    /// Returns the derived [`Xpriv`].
    ///
    /// # Arguments
    /// * `path` - the [`DerivationPath`] to be used to derive the [`Xpriv`]
    ///   from the master [`Xpriv`]
    ///
    /// # Errors
    ///
    /// This function will return an error if the derivation fails.
    fn xpriv_at(&self, path: DerivationPath) -> Result<Xpriv, Error> {
        self.master_xpriv
            .derive_priv(self.secp(), &path)
            .map_err(|_| Error::Derivation)
    }

    /// Returns the derived [`Xpub`].
    ///
    /// # Arguments
    /// * `path` - the [`DerivationPath`] to be used to derive the [`Xpub`]
    ///   from the master [`Xpub`]
    ///
    /// # Errors
    ///
    /// This function will return an error if the derivation fails.
    fn xpub_at(&self, path: DerivationPath) -> Result<Xpub, Error> {
        let xpriv = self.xpriv_at(path)?;
        Ok(Xpub::from_priv(self.secp(), &xpriv))
    }

    /// Returns a copy of the mnemonic if not None
    fn mnemonic(&self) -> Option<Mnemonic> {
        self.mnemonic.clone()
    }

    /// Returns the receive address at the given `index`.
    pub fn recv_addr_at(&self, index: u32) -> Address {
        self.address_at(&CoinPath {
            depth: 0,
            index: Some(index),
        })
        .expect("index is not none")
    }

    /// Returns the change address at the given `index`.
    pub fn change_addr_at(&self, index: u32) -> Address {
        self.address_at(&CoinPath {
            depth: 1,
            index: Some(index),
        })
        .expect("index is not none")
    }
}

impl JoinstrSigner for WpkhHotSigner {
    fn sign_input(&self, tx: &Transaction, input_data: Coin) -> Result<InputDataSigned, String> {
        self.sign(tx, input_data).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {

    use miniscript::bitcoin::{absolute, transaction::Version, Amount, Txid};

    use super::*;

    #[test]
    fn create_and_sign() {
        let signer = WpkhHotSigner::new(Network::Regtest).unwrap();

        let recv_script = signer
            .spk_at(&CoinPath {
                depth: 0,
                index: Some(11),
            })
            .unwrap();

        let input_data = Coin {
            txout: TxOut {
                value: Amount::from_btc(1.0).unwrap(),
                script_pubkey: recv_script,
            },
            outpoint: OutPoint {
                txid: Txid::from_str(
                    "000000000000000000032aea06ce8a8dd70127e86382b5ea68c7d810e8dbfc9b",
                )
                .unwrap(),
                vout: 0,
            },
            sequence: Sequence::MAX,
            coin_path: CoinPath {
                depth: 0,
                index: Some(11),
            },
        };

        let out1 = signer
            .spk_at(&CoinPath {
                depth: 0,
                index: Some(12),
            })
            .unwrap();
        let out2 = signer
            .spk_at(&CoinPath {
                depth: 0,
                index: Some(13),
            })
            .unwrap();

        let tx = Transaction {
            version: Version::ONE,
            lock_time: absolute::LockTime::from_height(0).unwrap(),
            input: Vec::new(),
            output: vec![
                TxOut {
                    value: Amount::from_btc(0.49).unwrap(),
                    script_pubkey: out1,
                },
                TxOut {
                    value: Amount::from_btc(0.49).unwrap(),
                    script_pubkey: out2,
                },
            ],
        };

        let _out_data = signer.sign(&tx, input_data).unwrap();
    }
}
