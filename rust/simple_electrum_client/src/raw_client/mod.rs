pub(crate) mod ssl_client;
pub(crate) mod tcp_client;

use std::{collections::HashMap, fmt::Display, net, thread, time::Duration};

use crate::electrum::{
    self,
    request::Request,
    response::{parse_str_response, Response},
};

use self::{ssl_client::SslClient, tcp_client::TcpClient};

// Using a 1 byte seek buffer
pub const PEEK_BUFFER_SIZE: usize = 10;

#[derive(Debug)]
pub enum Error {
    TcpStream(std::io::Error),
    SslStream(openssl::ssl::HandshakeError<net::TcpStream>),
    Electrum(electrum::Error),
    SslPeek,
    Mutex,
    SslConnector(std::io::Error),
    AlreadyConnected,
    NotConnected,
    NotConfigured,
    ShutDown,
    SetNonBlocking,
    SetBlocking,
    SerializeRequest,
    Batch,
}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl From<electrum::Error> for Error {
    fn from(value: electrum::Error) -> Self {
        Error::Electrum(value)
    }
}

#[derive(Debug, Default, Clone)]
pub enum Client {
    #[default]
    None,
    Tcp(TcpClient),
    Ssl(SslClient),
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

impl Client {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn tcp(self, url: &str, port: u16) -> Self {
        Self::new_tcp(url, port)
    }

    pub fn new_tcp(url: &str, port: u16) -> Self {
        Self::Tcp(TcpClient::default().url(url).port(port))
    }

    pub fn ssl(self, url: &str, port: u16) -> Self {
        Self::new_ssl(url, port)
    }

    pub fn new_ssl(url: &str, port: u16) -> Self {
        Self::Ssl(SslClient::default().url(url).port(port))
    }

    pub fn new_ssl_maybe(url: &str, port: u16, ssl: bool) -> Self {
        match ssl {
            true => Self::new_ssl(url, port),
            false => Self::new_tcp(url, port),
        }
    }

    pub fn read_timeout(mut self, timeout: Option<Duration>) -> Self {
        match &mut self {
            Client::None => {}
            Client::Tcp(c) => c.read_timeout = timeout,
            Client::Ssl(c) => c.read_timeout = timeout,
        }
        self
    }

    pub fn set_read_timeout(&mut self, timeout: Option<Duration>) -> Result<(), Error> {
        match self {
            Client::None => Err(Error::NotConfigured),
            Client::Tcp(c) => c.set_read_timeout(timeout),
            Client::Ssl(c) => c.set_read_timeout(timeout),
        }
    }

    pub fn write_timeout(mut self, timeout: Option<Duration>) -> Self {
        match &mut self {
            Client::None => {}
            Client::Tcp(c) => c.write_timeout = timeout,
            Client::Ssl(c) => c.write_timeout = timeout,
        }
        self
    }

    pub fn set_write_timeout(&mut self, timeout: Option<Duration>) -> Result<(), Error> {
        match self {
            Client::None => Err(Error::NotConfigured),
            Client::Tcp(c) => c.set_write_timeout(timeout),
            Client::Ssl(c) => c.set_write_timeout(timeout),
        }
    }

    pub fn verif_certificate(mut self, verif: bool) -> Self {
        let connected = self.is_connected();
        if let (
            Self::Ssl(SslClient {
                verif_certificate, ..
            }),
            false,
        ) = (&mut self, connected)
        {
            *verif_certificate = verif;
        }
        self
    }

    pub fn connect(&mut self) {
        self.try_connect().unwrap()
    }

    pub fn is_connected(&self) -> bool {
        match self {
            Client::None => false,
            Client::Tcp(c) => c.is_connected(),
            Client::Ssl(c) => c.is_connected(),
        }
    }

    pub fn try_connect(&mut self) -> Result<(), Error> {
        match self {
            Client::None => Err(Error::NotConfigured),
            Client::Tcp(c) => c.try_connect(),
            Client::Ssl(c) => c.try_connect(),
        }
    }

    pub fn try_connect_retry(&mut self, retry: usize, delay: Duration) -> Result<(), Error> {
        let mut result = self.try_connect();
        let mut count = 0;
        loop {
            match result {
                e @ Err(Error::NotConfigured) => return e,
                e @ Err(_) => {
                    thread::sleep(delay);
                    count += 1;
                    if count > retry {
                        return e;
                    }
                    result = self.try_connect();
                }
                ok => return ok,
            }
        }
    }

    pub fn send(&mut self, request: &Request) {
        self.try_send(request).unwrap();
    }

    pub fn send_str(&mut self, request: &str) {
        self.try_send_str(request).unwrap();
    }

    pub fn try_send_batch(&mut self, requests: Vec<&Request>) -> Result<(), Error> {
        let batch = serde_json::to_string(&requests).map_err(|_| Error::Batch)?;
        self.try_send_str(&batch)
    }

    pub fn try_send(&mut self, request: &Request) -> Result<(), Error> {
        let s = serde_json::to_string(request).map_err(|_| Error::SerializeRequest)?;
        self.try_send_str(&s)
    }

    pub fn try_send_str(&mut self, request: &str) -> Result<(), Error> {
        match self {
            Client::None => Err(Error::NotConfigured),
            Client::Tcp(c) => {
                if let Some(stream) = c.stream.as_mut() {
                    let mut stream = stream.lock().map_err(|_| Error::Mutex)?;
                    TcpClient::send(&mut stream, request)
                } else {
                    Err(Error::NotConnected)
                }
            }
            Client::Ssl(c) => {
                if let Some(stream) = c.stream.as_mut() {
                    let mut stream = stream.lock().map_err(|_| Error::Mutex)?;
                    SslClient::send(&mut stream, request)
                } else {
                    Err(Error::NotConnected)
                }
            }
        }
    }

    pub fn recv(&mut self, index: &HashMap<usize, Request>) -> Result<Vec<Response>, Error> {
        let raw = self.recv_str()?;
        Ok(parse_str_response(&raw, index)?)
    }

    pub fn recv_str(&mut self) -> Result<String, Error> {
        match self {
            Client::None => Err(Error::NotConfigured),
            Client::Tcp(c) => {
                if let Some(stream) = c.stream.as_mut() {
                    let mut stream = stream.lock().map_err(|_| Error::Mutex)?;
                    TcpClient::read(&mut stream)
                } else {
                    Err(Error::NotConnected)
                }
            }
            Client::Ssl(c) => {
                if let Some(stream) = c.stream.as_mut() {
                    let mut stream = stream.lock().map_err(|_| Error::Mutex)?;
                    SslClient::read(&mut stream)
                } else {
                    Err(Error::NotConnected)
                }
            }
        }
    }

    pub fn try_recv(
        &mut self,
        index: &HashMap<usize, Request>,
    ) -> Result<Option<Vec<Response>>, Error> {
        let raw = self.try_recv_str()?;
        if let Some(rr) = raw {
            Ok(Some(parse_str_response(&rr, index)?))
        } else {
            Ok(None)
        }
    }

    pub fn try_recv_str(&mut self) -> Result<Option<String>, Error> {
        match self {
            Client::None => Err(Error::NotConfigured),
            Client::Tcp(c) => {
                if let Some(stream) = c.stream.as_mut() {
                    let mut stream = stream.lock().map_err(|_| Error::Mutex)?;
                    TcpClient::try_read(&mut stream)
                } else {
                    Err(Error::NotConnected)
                }
            }
            Client::Ssl(c) => {
                if let Some(stream) = c.stream.as_mut() {
                    let mut stream = stream.lock().map_err(|_| Error::Mutex)?;
                    SslClient::try_read(&mut stream)
                } else {
                    Err(Error::NotConnected)
                }
            }
        }
    }

    pub fn close(&mut self) -> Result<(), Error> {
        match self {
            Client::None => Ok(()),
            Client::Tcp(c) => c.close(),
            Client::Ssl(c) => c.close(),
        }
    }
}
