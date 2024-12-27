mod error;
pub use error::Error;
use std::{
    collections::HashSet,
    fmt::{Debug, Display},
};

use crate::nostr::InputDataSigned;
use miniscript::bitcoin::{
    absolute, transaction::Version, Address, Amount, OutPoint, Psbt, Transaction, TxOut,
};

const BACKEND_RETRY: usize = 3;

pub trait BitcoinBackend {
    type Error: Into<Error>;
    fn address_already_used(&mut self, addr: &Address) -> Result<bool, Self::Error>;
    fn get_outpoint_value(&mut self, outpoint: OutPoint) -> Result<Option<Amount>, Self::Error>;
}

#[derive(Debug)]
pub struct CoinJoin<'a, C: BitcoinBackend> {
    /// List of outputs to be filled in the initial PSBT
    outputs: Vec<Address>,
    /// Contain the initial PSBT after all outputs added
    psbt: Option<Psbt>,
    /// List of PSBTs to be aggregated in the final psbt
    inputs: Vec<InputDataSigned>,
    /// The final transaction
    pub(crate) tx: Option<Transaction>,
    /// Min number of peer for an initial PSBT to be created
    min_peer: usize,
    /// The amount that will be used for all outputs
    denomination: Amount,
    /// Min feerate for the coinjoin to be considered broadcastable
    /// in sats/vb
    fee: usize,
    /// Electrum client, used to check input amount and addresses
    /// already used
    client: Option<&'a mut C>,
}

impl<'a, C> CoinJoin<'a, C>
where
    C: BitcoinBackend,
{
    /// Create a new CoinJoin instance.
    ///
    /// # Arguments
    /// * `denomination` - The coinjoin denomination
    /// * `client` - An optionnal electrum client, to be used for address reuse
    ///     and input amount check.
    pub fn new(denomination: Amount, client: Option<&'a mut C>) -> Self {
        CoinJoin {
            psbt: None,
            inputs: Vec::new(),
            tx: None,
            outputs: Vec::new(),
            min_peer: 5,
            denomination,
            fee: 2,
            client,
        }
    }

    /// Set the minimum of peer to consider for do a coinjoin.
    pub fn min_peer(mut self, peer: usize) -> Self {
        self.min_peer = peer;
        self
    }

    /// Set the fee rate in sats/vb
    pub fn fee(mut self, fee: usize) -> Self {
        self.fee = fee;
        self
    }

    /// Mapping to [`CoinJoin::generate_psbt()`] using builder pattern.
    pub fn generate(mut self) -> Result<Self, Error> {
        self.generate_psbt()?;
        Ok(self)
    }

    /// Add one address ouput to the coinjoin
    pub fn output(mut self, addr: Address) -> Self {
        self.add_output(addr);
        self
    }

    /// Add an electrum client
    pub fn set_client(&mut self, client: &'a mut C) {
        if self.client.is_none() {
            self.client = Some(client);
        }
    }

    /// Drop the current electrum client
    pub fn drop_client(&mut self) {
        self.client = None;
    }

    /// Add one address ouput to the coinjoin
    pub fn add_output(&mut self, addr: Address) {
        if !self.outputs.contains(&addr) {
            self.outputs.push(addr);
        }
    }

    /// Return length of [`CoinJoin::outputs`]
    pub fn outputs_len(&self) -> usize {
        self.outputs.len()
    }

    /// Return length of [`CoinJoin::inputs`]
    pub fn inputs_len(&self) -> usize {
        self.inputs.len()
    }

    /// Generate the initial psbt containing only ouputs.
    ///
    /// # Error if
    ///
    /// - the PSBT have already been generated
    /// - there is less addresses than defined in [`CoinJoin::min_peer`]
    /// - if an [`ElectrumClient`] is provided and of of the address have already been used
    pub fn generate_psbt(&mut self) -> Result<(), Error> {
        // move addresses into a HashSet in order to remove duplicates
        let addresses: HashSet<_> = self.outputs.clone().into_iter().collect();
        log::debug!("CoinJoin.generate_psbt(): outputs-> {:#?}", self.outputs);

        if addresses.len() < self.min_peer {
            return Err(Error::NotEnoughPeers(addresses.len(), self.min_peer));
        } else if self.psbt.is_some() {
            return Err(Error::InitPsbtExists);
        }

        // if an electrum client is provided, we check if address have been already used in order
        // to avoid address reuse
        let mut retry = 0;
        if let Some(client) = self.client.as_mut() {
            for addr in &addresses {
                // if client.address_already_used(addr) {
                // }
                loop {
                    match client.address_already_used(addr) {
                        Ok(used) => {
                            if used {
                                return Err(Error::AddressReuse);
                            } else {
                                break;
                            }
                        }
                        Err(e) => {
                            if retry > BACKEND_RETRY {
                                return Err(e.into());
                            } else {
                                retry += 1;
                                continue;
                            }
                        }
                    }
                }
            }
        }

        // process outputs
        let mut output: Vec<_> = addresses
            .iter()
            .map(|a| TxOut {
                value: self.denomination,
                script_pubkey: a.script_pubkey(),
            })
            .collect();

        // lexicographical sorting (BIP69)
        // as all the amount have the same value we only sort by script_pubkey
        // as it cannot have duplicate because previously checked
        output.sort_by(|a, b| a.script_pubkey.cmp(&b.script_pubkey));

        let tx = Transaction {
            version: Version::TWO,
            lock_time: absolute::LockTime::ZERO,
            input: Vec::new(),
            output,
        };

        self.psbt = Some(Psbt::from_unsigned_tx(tx).map_err(|_| Error::TxToPsbt)?);
        Ok(())
    }

    /// Return the PSBT unsigned transaction if present
    pub fn unsigned_tx(&self) -> Option<Transaction> {
        self.psbt.as_ref().map(|psbt| psbt.unsigned_tx.clone())
    }

    /// Add a new signed input received from a peer
    ///
    /// # Error if:
    ///
    /// - this input is already registered
    /// - an [`ElectrumClient`] is provided and the input amount supplyed by
    ///   the peer not match w/ the on-chain amount
    /// - an [`ElectrumClient`] is provided and the input does not exists
    pub fn add_input(&mut self, input: InputDataSigned) -> Result<(), Error> {
        // verify we not already register this input
        for i in self.inputs.as_slice() {
            if i.txin.previous_output == input.txin.previous_output {
                return Err(Error::DoubleSpend);
            }
        }

        // If an electrum client is provided, we verify our peer isn't lying
        // about the input value
        let mut retry = 0;
        if let Some(client) = self.client.as_mut() {
            loop {
                match client.get_outpoint_value(input.txin.previous_output) {
                    Ok(Some(amount)) => {
                        if let Some(peer_amount) = input.amount {
                            if amount != peer_amount {
                                return Err(Error::InputValueNotMatch);
                            }
                        }
                        break;
                    }
                    Ok(None) => {
                        return Err(Error::InputDoesNotExists);
                    }
                    Err(e) => {
                        if retry > BACKEND_RETRY {
                            return Err(e.into());
                        } else {
                            retry += 1;
                            continue;
                        }
                    }
                }
            }
        }

        // TODO: try to check the signature

        self.inputs.push(input);
        Ok(())
    }

    /// Generate the final Transaction
    ///
    /// Try to generate the final Transaction by aggregating received signed input to the
    /// `unsigned_tx` contained in [`CoinJoin::psbt`].
    ///
    /// # Arguments
    ///
    /// * `dry_run` - option to only produce a 'simulation'. If set to true, the produced
    ///   Transaction will not be stored in [`CoinJoin::tx`] but returned as Ok(Some(tx)).
    ///
    /// # Error if
    ///
    /// - [`ConJoin::psbt`] have not yet been generated
    /// - [`ConJoin::tx`] have already been generated
    /// - total input amount < (total_output + fees)
    pub fn generate_tx(&mut self, dry_run: bool) -> Result<Option<Transaction>, Error>
    where
        <C as crate::coinjoin::BitcoinBackend>::Error: Display,
    {
        // no sanity check for dry run
        let fee = if !dry_run {
            if self.tx.is_some() {
                return Err(Error::TxAlreadyFinalyzed);
            }
            let out_amount = if let Some(psbt) = self.psbt.as_ref() {
                psbt.unsigned_tx
                    .output
                    .iter()
                    .map(|o| o.value)
                    .sum::<Amount>()
            } else {
                return Err(Error::InitPsbtNotCreated);
            };
            let mut inp_amount = Amount::ZERO;
            for inp in self.inputs.iter() {
                if let Some(client) = self.client.as_mut() {
                    match client.get_outpoint_value(inp.txin.previous_output) {
                        Ok(Some(amount)) => inp_amount += amount,
                        Ok(None) => {
                            log::error!("Coinjoin::generate_tx() fail to verify input amount: the input did not exists");
                            return Err(Error::FailVerifyAmount);
                        }
                        Err(e) => {
                            log::error!(
                                "Coinjoin::generate_tx() fail to verify input amount: {}",
                                e
                            );
                            return Err(Error::FailVerifyAmount);
                        }
                    }
                } else if let Some(amount) = inp.amount {
                    inp_amount += amount;
                } else {
                    log::error!("Coinjoin::generate_tx() input amount missing and no electrum client available to verify");
                    return Err(Error::AmountMissing);
                }
            }

            if inp_amount <= out_amount {
                return Err(Error::InputAmountTooLow);
            }

            let fee = inp_amount - out_amount;

            // sort lexically
            self.inputs.sort_by(|a, b| {
                a.txin
                    .previous_output
                    .txid
                    .cmp(&b.txin.previous_output.txid)
                    .then(
                        a.txin
                            .previous_output
                            .vout
                            .cmp(&b.txin.previous_output.vout),
                    )
            });
            Some(fee)
        } else {
            None
        };

        let mut tx = if let Some(psbt) = self.psbt.as_ref() {
            psbt.unsigned_tx.clone()
        } else {
            return Err(Error::InitPsbtNotCreated);
        };
        for i in self.inputs.as_slice() {
            tx.input.push(i.txin.clone());
        }

        // if not dry_run
        if let Some(fee) = fee {
            let tx_weight = tx.weight().to_wu();
            if (((fee.to_sat() as f64) / (tx_weight as f64)) < (self.fee as f64)) && !dry_run {
                return Err(Error::FeeTooLow(self.fee as u64, tx_weight, fee.to_sat()));
            }
            self.tx = Some(tx);
            Ok(None)
        } else {
            Ok(Some(tx))
        }
    }

    pub fn tx(&self) -> Option<Transaction> {
        self.tx.clone()
    }
}

#[cfg(test)]
pub mod tests {}
