use std::{
    fmt::Debug,
    io::ErrorKind,
    sync::mpsc::{self, Receiver, Sender},
    time::{Duration, SystemTime},
};

use nostr::{
    event::{Event, EventBuilder, Kind, Tag},
    key::{Keys, PublicKey},
    message::{ClientMessage, RawRelayMessage, RelayMessage, SubscriptionId},
    nips::nip04,
    types::{Filter, Timestamp},
    util::JsonUtil,
};
use websocket::{
    stream::sync::NetworkStream, sync::Client, url::ParseError, ClientBuilder, OwnedMessage,
    WebSocketError,
};

pub use nostr;
pub use websocket;

const PING_INTERVAL: u64 = 5; // ping interval in seconds

#[derive(Debug)]
pub enum Error {
    WebSocket(WebSocketError),
    Parse(ParseError),
    Listen,
    Send,
    Receive,
    NonBlocking,
    NotConnected,
    KeysMissing,
    ArgMissing,
    Nip04Encrypt,
    Nip04Decrypt,
    NotNip04,
    SignEvent,
    ConnectionClosed,
    RawRelayMessage,
    RelayMessage,
}

impl From<WebSocketError> for Error {
    fn from(value: WebSocketError) -> Self {
        Self::WebSocket(value)
    }
}

impl From<ParseError> for Error {
    fn from(value: ParseError) -> Self {
        Self::Parse(value)
    }
}

type Message = String;

#[derive(Debug)]
pub enum SendMsg {
    Msg(Message),
    Stop,
}

#[derive(Debug)]
pub enum RecvMsg {
    Close,
    Msg(Message),
}

pub struct WsClient {
    client: Option<Client<Box<dyn NetworkStream + Send>>>,
    sender: Sender<SendMsg>,
    ws_receiver: Option<Receiver<SendMsg>>,
    receiver: Receiver<RecvMsg>,
    ws_sender: Option<Sender<RecvMsg>>,
    connected: bool,
    relay: String,
    keys: Keys,
}

impl Debug for WsClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WsClient")
            .field("connected", &self.connected)
            .field("relay", &self.relay)
            .field("keys", &self.keys)
            .finish()
    }
}

#[derive(Debug, Default, Clone)]
pub struct WsClientBuilder {
    relay: Option<String>,
    keys: Option<Keys>,
}

impl WsClientBuilder {
    pub fn relay<T: Into<String>>(mut self, relay: T) -> Self {
        self.set_relay(relay);
        self
    }

    pub fn set_relay<T: Into<String>>(&mut self, relay: T) {
        self.relay = Some(relay.into());
    }

    pub fn get_relay(&self) -> Option<String> {
        self.relay.clone()
    }

    pub fn keys(mut self, keys: Keys) -> Self {
        self.keys = Some(keys);
        self
    }

    pub fn set_keys(&mut self, keys: Keys) {
        self.keys = Some(keys);
    }

    pub fn get_keys(&self) -> Option<&Keys> {
        self.keys.as_ref()
    }

    pub fn connect(self) -> Result<WsClient, Error> {
        let (url, keys) = if let (Some(url), Some(keys)) = (self.relay, self.keys) {
            (url, keys)
        } else {
            return Err(Error::ArgMissing);
        };
        let client = ClientBuilder::new(&url)?.connect(None)?;
        client
            .set_nonblocking(true)
            .map_err(|_| Error::NonBlocking)?;
        let (sender, ws_receiver) = mpsc::channel();
        let (ws_sender, receiver) = mpsc::channel();
        let mut client = WsClient {
            client: Some(client),
            sender,
            ws_receiver: Some(ws_receiver),
            receiver,
            ws_sender: Some(ws_sender),
            connected: false,
            relay: url,
            keys,
        };
        client.listen()?;
        Ok(client)
    }
}

impl WsClient {
    #[allow(clippy::new_ret_no_self)]
    pub fn new() -> WsClientBuilder {
        WsClientBuilder::default()
    }

    pub fn pubkey(&self) -> PublicKey {
        self.keys.public_key
    }

    fn listen(&mut self) -> Result<(), Error> {
        if let (Some(client), Some(sender), Some(receiver)) = (
            self.client.take(),
            self.ws_sender.take(),
            self.ws_receiver.take(),
        ) {
            std::thread::spawn(|| listen(client, sender, receiver));
            self.connected = true;
            Ok(())
        } else {
            Err(Error::Listen)
        }
    }

    pub fn encrypt<T>(&mut self, receiver: &PublicKey, content: T) -> Result<String, Error>
    where
        T: AsRef<[u8]>,
    {
        nip04::encrypt(self.get_keys().secret_key(), receiver, content)
            .map_err(|_| Error::Nip04Encrypt)
    }

    pub fn decrypt_dm(&mut self, mut event: Event) -> Result<Event, Error> {
        if event.kind != Kind::EncryptedDirectMessage {
            return Err(Error::NotNip04);
        }
        let content = self.decrypt(&event.pubkey, event.content)?;
        event.content = content;
        Ok(event)
    }

    pub fn decrypt(&mut self, event_pubkey: &PublicKey, content: String) -> Result<String, Error> {
        nip04::decrypt(self.get_keys().secret_key(), event_pubkey, content)
            .map_err(|_| Error::Nip04Decrypt)
    }

    pub fn subscribe_dm(&mut self) -> Result<(), Error> {
        self.is_connected()?;
        let filter = Filter::new()
            .kind(Kind::EncryptedDirectMessage)
            .pubkey(self.get_keys().public_key());
        let msg = nostr::ClientMessage::req(SubscriptionId::generate(), vec![filter]);
        self.send_raw(msg.as_json())?;
        Ok(())
    }

    pub fn subscribe_pool(&mut self, back: u64) -> Result<(), Error> {
        self.is_connected()?;
        let since = Timestamp::now() - Timestamp::from_secs(back);
        let filter = Filter::new().kind(Kind::Custom(2022)).since(since);
        let msg = nostr::ClientMessage::req(SubscriptionId::generate(), vec![filter]);
        self.send_raw(msg.as_json())?;
        Ok(())
    }

    pub fn send_dm<T: Into<String>>(
        &mut self,
        content: T,
        receiver: &PublicKey,
    ) -> Result<(), Error> {
        let content = self.encrypt(receiver, content.into())?;
        let dm = EventBuilder::new(
            Kind::EncryptedDirectMessage,
            content,
            vec![Tag::public_key(*receiver)],
        );
        self.post_event(dm)
    }

    fn send_raw(&mut self, msg: Message) -> Result<(), Error> {
        self.is_connected()?;
        self.sender.send(SendMsg::Msg(msg)).map_err(|_| Error::Send)
    }

    fn try_receive_raw(&mut self) -> Result<Option<RecvMsg>, Error> {
        self.is_connected()?;
        let msg = match self.receiver.try_recv() {
            Ok(m) => Ok(Some(m)),
            Err(e) => match e {
                mpsc::TryRecvError::Empty => Ok(None),
                mpsc::TryRecvError::Disconnected => Err(Error::Receive),
            },
        };
        if let Ok(Some(RecvMsg::Close)) = msg {
            self.connected = false;
        }
        msg
    }

    pub fn try_receive(&mut self) -> Result<Option<Event>, Error> {
        match self.try_receive_raw()? {
            Some(m) => match m {
                RecvMsg::Close => Err(Error::ConnectionClosed),
                RecvMsg::Msg(t) => match RawRelayMessage::from_json(t) {
                    Ok(rrm) => match RelayMessage::try_from(rrm) {
                        Ok(rm) => match rm {
                            RelayMessage::Event { event, .. } => {
                                #[allow(deprecated)]
                                if event.kind() == Kind::EncryptedDirectMessage {
                                    let event = self.decrypt_dm(*event)?;
                                    Ok(Some(event))
                                } else {
                                    Ok(Some(*event))
                                }
                            }
                            RelayMessage::Auth { .. } => {
                                log::error!("unexpected auth message");
                                Ok(None)
                            }
                            _ => Ok(None),
                        },
                        Err(_) => Err(Error::RelayMessage),
                    },
                    Err(_) => Err(Error::RawRelayMessage),
                },
            },
            None => Ok(None),
        }
    }

    pub fn stop(&mut self) {
        if self.connected {
            self.connected = false;
            _ = self.sender.send(SendMsg::Stop);
        }
    }

    pub fn get_relay(&self) -> String {
        self.relay.clone()
    }

    pub fn is_connected(&self) -> Result<(), Error> {
        if self.connected {
            Ok(())
        } else {
            Err(Error::NotConnected)
        }
    }

    pub fn get_keys(&self) -> &Keys {
        &self.keys
    }

    pub fn post_event(&mut self, event: EventBuilder) -> Result<(), Error> {
        self.is_connected()?;
        let event = event
            .to_event(self.get_keys())
            .map_err(|_| Error::SignEvent)?;
        let msg = ClientMessage::event(event);
        log::debug!("_post_event() msg: {:?}", msg);
        self.send_raw(msg.as_json())
    }
}

impl Drop for WsClient {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn listen(
    mut client: Client<Box<dyn NetworkStream + Send>>,
    sender: Sender<RecvMsg>,
    receiver: Receiver<SendMsg>,
) {
    let mut last_ping = SystemTime::now();
    let mut last_pong = SystemTime::now();
    let mut ping_nonce = 0u8;
    loop {
        let mut wait = true;
        match receiver.try_recv() {
            Ok(m) => match m {
                SendMsg::Msg(m) => {
                    wait = false;
                    if let Err(e) = client.send_message(&websocket::Message::text(m)) {
                        log::error!("listen(): fail to send message: {:?}", e);
                    }
                }
                SendMsg::Stop => return,
            },
            Err(mpsc::TryRecvError::Empty) => {}
            _ => return,
        }

        match client.recv_message() {
            Ok(m) => {
                wait = false;
                match m {
                    OwnedMessage::Text(m) => {
                        log::debug!("recv text: {:?}", m);
                        let _ = sender.send(RecvMsg::Msg(m));
                    }
                    OwnedMessage::Binary(m) => {
                        log::error!("listen() unexpected binary message {:?}", m);
                    }
                    OwnedMessage::Close(_) => {
                        log::debug!("recv: Close ");
                        sender.send(RecvMsg::Close).expect("main thread panicked");
                    }
                    OwnedMessage::Ping(nonce) => {
                        _ = client.send_message(&OwnedMessage::Pong(nonce));
                    }
                    OwnedMessage::Pong(_) => {
                        last_pong = SystemTime::now();
                    }
                }
            }
            Err(e) => match e {
                WebSocketError::ProtocolError(_e) => {
                    // FIXME:: why do we receive a bunch of protocols errors at startup?
                    // log::error!("ProtocolError: {:?}", e);
                }
                WebSocketError::DataFrameError(_e) => {
                    // FIXME: why do we receive a bunch of "Expected unmasked data frame" at startup?
                    // log::error!("DataFrameError: {:?}", e);
                }
                WebSocketError::NoDataAvailable => {}
                WebSocketError::IoError(e) => {
                    if e.kind() == ErrorKind::WouldBlock {
                    } else {
                        log::error!("{:?}", e);
                    }
                }
                WebSocketError::Utf8Error(e) => {
                    log::error!("{:?}", e);
                }
                WebSocketError::Other(e) => {
                    log::error!("{:?}", e);
                }
            },
        }

        if SystemTime::now()
            .duration_since(last_ping)
            .expect("valid duration")
            > Duration::from_secs(PING_INTERVAL)
        {
            last_ping = SystemTime::now();
            ping_nonce = ping_nonce.wrapping_add(1);
            _ = client.send_message(&OwnedMessage::Ping(vec![ping_nonce]));
        }

        if SystemTime::now()
            .duration_since(last_pong)
            .expect("valid duration")
            > Duration::from_secs(3 * PING_INTERVAL)
        {
            _ = sender.send(RecvMsg::Close);
            return;
        }

        if wait {
            std::thread::sleep(Duration::from_micros(50));
        }
    }
}
