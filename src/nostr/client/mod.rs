mod error;
pub use error::Error;

use std::{str::FromStr, time::Duration};

use nostr_sdk::{
    nips::nip04, Client, Event, EventBuilder, Filter, Keys, Kind, Options, PublicKey,
    RelayPoolNotification, Tag, Timestamp,
};
use tokio::sync::broadcast;

use super::{Pool, PoolMessage};

#[derive(Debug, Default)]
pub struct NostrClient {
    keys: Option<Keys>,
    relays: Vec<String>,
    client: Option<Client>,
    nostr_receiver: Option<broadcast::Receiver<RelayPoolNotification>>,
    pub name: String,
}

impl NostrClient {
    pub fn new(name: &str) -> NostrClient {
        NostrClient {
            name: name.into(),
            ..Default::default()
        }
    }

    pub fn relay(mut self, url: String) -> Result<Self, Error> {
        if self.client.is_none() {
            self.relays.push(url);
            Ok(self)
        } else {
            Err(Error::AlreadyConnected)
        }
    }

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

    pub fn get_relays(&self) -> &Vec<String> {
        &self.relays
    }

    pub fn keys(mut self, keys: Keys) -> Result<Self, Error> {
        if self.client.is_none() {
            self.keys = Some(keys);
            Ok(self)
        } else {
            Err(Error::AlreadyConnected)
        }
    }

    pub fn nostr_receiver(&mut self) -> Option<broadcast::Receiver<RelayPoolNotification>> {
        self.nostr_receiver.take()
    }

    pub async fn connect_nostr(&mut self) -> Result<(), Error> {
        let opts = Options::new()
            .skip_disconnected_relays(true)
            .connection_timeout(Some(Duration::from_secs(10)))
            .send_timeout(Some(Duration::from_secs(5)));

        let client = Client::with_opts(self.get_keys()?, opts);
        #[allow(deprecated)]
        match client.add_relays(self.relays.as_slice()).await {
            Ok(_) => {
                client.connect().await;
                self.nostr_receiver = Some(client.notifications());
                self.client = Some(client);
                self.subscribe_dm().await?;
                Ok(())
            }
            Err(e) => Err(e.into()),
        }
    }

    pub fn is_connected(&self) -> Result<(), Error> {
        if self.client.is_some() {
            Ok(())
        } else {
            Err(Error::NotConnected)
        }
    }

    pub fn client(&self) -> Result<&Client, Error> {
        self.client.as_ref().ok_or(Error::NotConnected)
    }

    pub fn get_keys(&self) -> Result<&Keys, Error> {
        self.keys.as_ref().ok_or(Error::KeysMissing)
    }

    pub async fn post_event(&self, event: EventBuilder) -> Result<(), Error> {
        self.is_connected()?;
        let event = event.to_event(self.get_keys()?)?;
        // log::warn!("NostrClient.post_event({event:?})");
        self.client()?.send_event(event).await?;
        Ok(())
    }

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

    pub async fn send_pool_message(&self, npub: &PublicKey, msg: PoolMessage) -> Result<(), Error> {
        let clear_content = msg.to_string()?;
        log::info!("NostrClient.send_pool_message(): {:#?}", clear_content);
        self.send_dm(npub, clear_content).await
    }

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

    pub async fn subscribe_pools(&self, back: u64) -> Result<(), Error> {
        let client = self.client()?;
        let since = Timestamp::now() - Timestamp::from_secs(back);
        let filter = Filter::new().kind(Kind::Custom(2022)).since(since);
        client.subscribe(vec![filter], None).await?;
        Ok(())
    }

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
