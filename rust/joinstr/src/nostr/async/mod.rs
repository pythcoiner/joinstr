use std::{str::FromStr, time::Duration};

use nostr_sdk::{
    nips::nip04, Client, Event, EventBuilder, Filter, Keys, Kind, Options, PublicKey,
    RelayPoolNotification, Tag, Timestamp,
};

use tokio::sync::broadcast;

use crate::nostr::{error::Error, Pool, PoolMessage};

#[derive(Debug, Default)]
pub struct NostrClient {
    keys: Option<Keys>,
    relays: Vec<String>,
    client: Option<Client>,
    nostr_receiver: Option<broadcast::Receiver<RelayPoolNotification>>,
    pub name: String,
}

impl NostrClient {
    /// Create a new nostr client.
    ///
    /// # Arguments
    /// * `name` - Name of the [`NostrClient`] instance (use for debug logs), can
    ///   be an empty &str.
    pub fn new(name: &str) -> NostrClient {
        NostrClient {
            name: name.into(),
            ..Default::default()
        }
    }

    /// Add a nostr relay url to [`NostrClient::relays`]
    ///
    /// # Errors
    ///
    /// This function will return an error if the client is already connected
    ///   to some relays.
    pub fn relay(mut self, url: String) -> Result<Self, Error> {
        if self.client.is_none() {
            self.relays.push(url);
            Ok(self)
        } else {
            Err(Error::AlreadyConnected)
        }
    }

    /// Copy the given list of relays into [`NostrClient::relays`] .
    ///
    /// # Errors
    ///
    /// This function will return an error if [`NostrClient::relays`]
    ///   if the client is already connected to some relays.
    pub fn relays(mut self, relays: &Vec<String>) -> Result<Self, Error> {
        if self.client.is_none() {
            for url in relays {
                self.relays.push(url.into());
            }
            Ok(self)
        } else {
            Err(Error::AlreadyConnected)
        }
    }

    /// Set the nostr key pair of this client.
    ///
    /// # Errors
    ///
    /// This function will return an error if the client is already
    ///   connected to some relays.
    pub fn keys(mut self, keys: Keys) -> Result<Self, Error> {
        if self.client.is_none() {
            self.keys = Some(keys);
            Ok(self)
        } else {
            Err(Error::AlreadyConnected)
        }
    }

    /// Returns a reference to [`NostrClient::relays`].
    ///
    /// # Errors
    ///
    /// This function will return an error if [`NostrClient::relays`]
    ///   if the client is already connected to some relays.
    pub fn get_relays(&self) -> &Vec<String> {
        &self.relays
    }

    /// Take the receiver end of the notification channels from the inner client.
    pub fn nostr_receiver(&mut self) -> Option<broadcast::Receiver<RelayPoolNotification>> {
        self.nostr_receiver.take()
    }

    /// Connect to nostr relays defined in [`NostrClient::relays`].
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    ///   - no nostr keypair have been set.
    ///   - adding a relay fails
    ///   - suscribing to NIP04 Dms fails
    pub async fn connect_nostr(&mut self) -> Result<(), Error> {
        let opts = Options::new()
            .skip_disconnected_relays(true)
            .connection_timeout(Some(Duration::from_secs(10)))
            .send_timeout(Some(Duration::from_secs(5)));

        let client = Client::with_opts(self.get_keys()?, opts);
        // TODO: Do not use a deprecated method
        #[allow(deprecated)]
        match client.add_relays(self.relays.as_slice()).await {
            Ok(_) => {
                client.connect().await;
                self.nostr_receiver = Some(client.notifications());
                self.client = Some(client);
                self.subscribe_dm().await?;
                Ok(())
            }
            // FIXME: we should not error if a single relay cannot be added
            // but return a map of relays status (Connected/Failed) instead.
            Err(e) => Err(e.into()),
        }
    }

    /// Utility function, will error if the client is not connected.
    pub fn is_connected(&self) -> Result<(), Error> {
        if self.client.is_some() {
            Ok(())
        } else {
            Err(Error::NotConnected)
        }
    }

    /// Returns a ref to [`NostrClient::client`]
    ///
    /// # Errors
    ///
    /// This function will return an error if not connected.
    pub fn client(&self) -> Result<&Client, Error> {
        self.client.as_ref().ok_or(Error::NotConnected)
    }

    /// Returns a ref to [`NostrClient::keys`]
    ///
    /// # Errors
    ///
    /// This function will return an error if the keypair has
    ///   not been set.
    pub fn get_keys(&self) -> Result<&Keys, Error> {
        self.keys.as_ref().ok_or(Error::KeysMissing)
    }

    /// Post a nostr event.
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    ///   - the client is not connected
    ///   - fail to send event.
    pub async fn post_event(&self, event: EventBuilder) -> Result<(), Error> {
        self.is_connected()?;
        let event = event.to_event(self.get_keys()?)?;
        self.client()?.send_event(event).await?;
        Ok(())
    }

    /// Send a NIP04 encrypted DM
    ///
    /// # Arguments
    /// * `npub` - nostr pubkey of the receiver
    /// * `content` - raw (unencrypted) message content as String
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    ///   - the client is not connected
    ///   - the client do not have signing keys
    ///   - encryption of the message fails
    ///   - sending the DM fails
    pub async fn send_dm(&self, npub: &PublicKey, content: String) -> Result<(), Error> {
        let client = self.client()?;
        let signer = client.signer().await?;
        log::warn!(
            "NostrClient({}).send_dm(): Sending \"{}\" to {} ",
            self.name,
            content,
            npub
        );
        let content = signer.nip04_encrypt(npub, content).await?;
        let dm = EventBuilder::new(
            Kind::EncryptedDirectMessage,
            content,
            vec![Tag::public_key(*npub)],
        );
        self.post_event(dm).await?;
        Ok(())
    }

    /// Send a [`PoolMessage`] wrapped into a NIP04 encrypted DM
    ///
    /// # Arguments
    /// * `npub` - nostr pubkey of the pool
    /// * `msg` - the PoolMessage to send
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    ///   - teh message cannot be serialized into String json payload
    ///   - sending the DM fails
    pub async fn send_pool_message(&self, npub: &PublicKey, msg: PoolMessage) -> Result<(), Error> {
        let clear_content = msg.to_string()?;
        log::debug!("NostrClient.send_pool_message(): {:#?}", clear_content);
        self.send_dm(npub, clear_content).await
    }

    /// Subscribe to notifications of NIP04 DMs thatare send tu the client pubkey
    ///
    /// # Errors
    ///
    /// This function will return an error if :
    ///   - the client is not connected
    ///   - the client does not have signing keys
    ///   - subscription fail
    pub async fn subscribe_dm(&self) -> Result<(), Error> {
        let client = self.client()?;
        let keys = self.get_keys()?;
        log::debug!(
            "NotrClient({}).subscribe_dm(): subscribe to DM @ {}",
            self.name,
            &keys.public_key().to_string()[0..6]
        );
        let filter = Filter::new()
            .kind(Kind::EncryptedDirectMessage)
            .pubkey(keys.public_key());

        client.subscribe(vec![filter], None).await?;
        Ok(())
    }

    /// Subscribe to notifications of NIP04 DMs thatare send tu the client pubkey
    ///
    /// # Arguments
    /// * `back` - the client will not receive notifications for pools that have been initiated
    ///   `back` seconds in the past.
    ///
    /// # Errors
    ///
    /// This function will return an error if :
    ///   - the client is not connected
    ///   - subscription fail
    pub async fn subscribe_pools(&self, back: u64) -> Result<(), Error> {
        let client = self.client()?;
        let since = Timestamp::now() - Timestamp::from_secs(back);
        let filter = Filter::new().kind(Kind::Custom(2022)).since(since);
        client.subscribe(vec![filter], None).await?;
        Ok(())
    }

    /// Try to poll notifications/events received by the client, will return:
    ///   - Some(event) if there is at list one event is in the channel, in this
    ///     case the message is remode from the channel.
    ///   - None if the channel is empty
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    ///   - the client is not connected
    ///   - the channel is closed
    pub fn receive_event(&mut self) -> Result<Option<Event>, Error> {
        if let Some(receiver) = self.nostr_receiver.as_mut() {
            match receiver.try_recv() {
                Ok(notif) => {
                    if let RelayPoolNotification::Event { event, .. } = notif {
                        log::info!(
                            "NostrClient({}).receive_event(@ {}): {:?}",
                            self.name,
                            self.get_keys()?.public_key(),
                            event
                        );
                        Ok(Some(*event))
                    } else {
                        Ok(None)
                    }
                }
                Err(e) => match e {
                    broadcast::error::TryRecvError::Empty => Ok(None),
                    _ => Err(Error::Disconnected),
                },
            }
        } else {
            Err(Error::NotConnected)
        }
    }

    /// Try to decrypt the payload of a NIP04 DM, the event will be returned
    ///   with its encrypted payload replaced by clear text one.
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    ///   - the event is not of Kind 04
    ///   - decryption fails
    pub fn decrypt_dm(&self, mut event: Event) -> Result<Event, Error> {
        if event.kind != Kind::EncryptedDirectMessage {
            return Err(Error::NotNip04);
        }
        let keys = self.get_keys()?;
        match nip04::decrypt(keys.secret_key(), &event.pubkey, event.content) {
            Ok(decrypted) => {
                event.content = decrypted;
                Ok(event)
            }
            _ => Err(Error::DmEncryption),
        }
    }

    /// Try to poll notifications/events received by the client and parse it as
    ///    a PoolMessage, will return:
    ///    - Some(PoolMessage) if there is a message in the channel
    ///    - None if the channel is empty
    ///
    /// Note: if the message is of type [`PoolMessage::Join`] and the pubkey is not
    ///   specified, we will replace None by the sender pubkey.
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    ///   - the client is not connected
    ///   - the channel is closed
    ///   - the the received event is not a NIP04
    ///   - the event cannot be parsed as a PoolMessage
    pub fn receive_pool_msg(&mut self) -> Result<Option<PoolMessage>, Error> {
        let event = self
            .receive_event()?
            .filter(|e| e.kind == Kind::EncryptedDirectMessage);

        Ok(if let Some(event) = event {
            let event = match self.decrypt_dm(event) {
                Ok(c) => c,
                Err(Error::DmEncryption) => {
                    log::error!(
                        "NostrClient({}).receive_pool_msg(): cannot decrypt DM!",
                        self.name
                    );
                    return Ok(None);
                }
                e => e?,
            };
            PoolMessage::from_str(&event.content).ok().map(|m| {
                // if the join request does not contain a pubkey to respond to, we respond to
                // sender
                if let PoolMessage::Join(None) = m {
                    PoolMessage::Join(Some(event.pubkey))
                } else {
                    m
                }
            })
        } else {
            None
        })
    }

    /// Try to poll notifications/events received by the client and parse it as
    ///    a Pool, it will return:
    ///    - Some(Pool) if there is a message in the channel
    ///    - None if the channel is empty or fail to parse as a Pool message
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    ///   - fails to receive event
    pub fn receive_pool_notification(&mut self) -> Result<Option<Pool>, Error> {
        let event = self
            .receive_event()?
            .filter(|e| e.kind == Kind::Custom(2022));
        if let Some(Event { content, .. }) = event {
            log::info!(
                "NostrClient({}).receive_pool_notification(): {:?}",
                self.name,
                content
            );
            Ok(serde_json::from_str(&content).ok())
        } else {
            Ok(None)
        }
    }
}
