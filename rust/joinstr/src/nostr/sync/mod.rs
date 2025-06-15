use std::{fmt::Debug, str::FromStr};

use simple_nostr_client::nostr::event::{Event, EventBuilder};
use simple_nostr_client::nostr::key::PublicKey;
use simple_nostr_client::nostr::Keys;
use simple_nostr_client::{WsClient, WsClientBuilder};

use crate::nostr::{error::Error, Pool, PoolMessage};

#[derive(Default)]
pub struct NostrClient {
    client: Option<WsClient>,
    builder: Option<WsClientBuilder>,
    pub name: String,
}

impl Debug for NostrClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NostrClient")
            .field("name", &self.name)
            .finish()
    }
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
            builder: Some(WsClient::new()),
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
        if let Some(builder) = self.builder.as_mut() {
            builder.set_relay(url);
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
        if let Some(builder) = self.builder.as_mut() {
            builder.set_keys(keys);
            Ok(self)
        } else {
            Err(Error::AlreadyConnected)
        }
    }

    /// Returns the relay url if available
    pub fn get_relay(&self) -> Option<String> {
        self.client.as_ref().map(|client| client.get_relay())
    }

    /// Connect to nostr relays defined in [`NostrClient::relays`].
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    ///   - no nostr keypair have been set.
    ///   - adding a relay fails
    ///   - suscribing to NIP04 Dms fails
    pub fn connect_nostr(&mut self) -> Result<(), Error> {
        if let Some(builder) = self.builder.take() {
            let mut client = builder.connect()?;
            client.subscribe_dm()?;
            self.client = Some(client);
            Ok(())
        } else {
            Err(Error::SyncClientBuilderMissing)
        }
    }

    /// Utility function, will error if the client is not connected.
    pub fn is_connected(&self) -> Result<(), Error> {
        if let Some(client) = &self.client {
            client.is_connected().map_err(|_| Error::NotConnected)
        } else {
            Err(Error::NotConnected)
        }
    }

    /// Returns a ref to [`NostrClient::client`]
    ///
    /// # Errors
    ///
    /// This function will return an error if not connected.
    pub fn client(&mut self) -> Result<&mut WsClient, Error> {
        self.client.as_mut().ok_or(Error::NotConnected)
    }

    /// Returns a ref to [`NostrClient::keys`]
    ///
    /// # Errors
    ///
    /// This function will return an error if the keypair has
    ///   not been set.
    pub fn get_keys(&self) -> Result<&Keys, Error> {
        if let Some(client) = &self.client {
            Ok(client.get_keys())
        } else if let Some(builder) = &self.builder {
            builder.get_keys().ok_or(Error::KeysMissing)
        } else {
            Err(Error::KeysMissing)
        }
    }

    /// Post a nostr event.
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    ///   - the client is not connected
    ///   - fail to send event.
    pub fn post_event(&mut self, event: EventBuilder) -> Result<(), Error> {
        self.client()?.post_event(event)?;
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
    pub fn send_dm(&mut self, npub: &PublicKey, content: String) -> Result<(), Error> {
        self.client()?.send_dm(content, npub)?;
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
    pub fn send_pool_message(&mut self, npub: &PublicKey, msg: PoolMessage) -> Result<(), Error> {
        let clear_content = msg.to_string()?;
        log::debug!("NostrClient.send_pool_message(): {:#?}", clear_content);
        self.send_dm(npub, clear_content)
    }

    /// Subscribe to notifications of NIP04 DMs thatare send tu the client pubkey
    ///
    /// # Errors
    ///
    /// This function will return an error if :
    ///   - the client is not connected
    ///   - the client does not have signing keys
    ///   - subscription fail
    pub async fn subscribe_dm(&mut self) -> Result<(), Error> {
        self.client()?.subscribe_dm()?;
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
    pub fn subscribe_pools(&mut self, back: u64) -> Result<(), Error> {
        self.client()?.subscribe_pool(back)?;
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
        let ev = self.client()?.try_receive()?;
        Ok(ev)
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
    pub fn try_receive_pool_msg(&mut self) -> Result<Option<PoolMessage>, Error> {
        Ok(if let Some(event) = self.client()?.try_receive()? {
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
        Ok(if let Some(event) = self.client()?.try_receive()? {
            Pool::try_from(event).ok()
        } else {
            None
        })
    }
}
