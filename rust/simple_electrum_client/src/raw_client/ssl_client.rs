use super::{Error, PEEK_BUFFER_SIZE};
use openssl::ssl::{self, SslConnector, SslMethod, SslVerifyMode};
use std::{
    io::{BufRead, BufReader, Write},
    net,
    sync::{Arc, Mutex},
    time::Duration,
};

type SslStream = Arc<Mutex<ssl::SslStream<net::TcpStream>>>;

#[derive(Debug)]
pub struct SslClient {
    url: String,
    port: u16,
    pub(crate) stream: Option<SslStream>,
    pub(crate) read_timeout: Option<Duration>,
    pub(crate) write_timeout: Option<Duration>,
    pub(crate) verif_certificate: bool,
}

impl Clone for SslClient {
    fn clone(&self) -> Self {
        Self {
            url: self.url.clone(),
            port: self.port,
            stream: self.stream.clone(),
            read_timeout: self.read_timeout,
            write_timeout: self.write_timeout,
            verif_certificate: self.verif_certificate,
        }
    }
}

impl Drop for SslClient {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

impl Default for SslClient {
    fn default() -> Self {
        Self {
            url: Default::default(),
            port: 50002,
            stream: None,
            read_timeout: None,
            write_timeout: None,
            verif_certificate: true,
        }
    }
}

impl SslClient {
    pub fn url(mut self, url: &str) -> Self {
        if !self.is_connected() {
            self.url = url.into();
        } else {
            log::error!("Cannot change url of a connected SslClient!")
        }
        self
    }

    pub fn port(mut self, port: u16) -> Self {
        if !self.is_connected() {
            self.port = port;
        } else {
            log::error!("Cannot change port of a connected TcpClient!")
        }
        self
    }

    pub fn is_connected(&self) -> bool {
        self.stream.is_some()
    }

    pub fn try_connect(&mut self) -> Result<(), Error> {
        let url = format!("{}:{}", self.url, self.port);
        let mut ssl = SslConnector::builder(SslMethod::tls()).unwrap();
        // do not verify for self-signed certs
        if !self.verif_certificate {
            ssl.set_verify(SslVerifyMode::NONE);
        }
        let ssl = ssl.build();
        let stream = net::TcpStream::connect(url).map_err(Error::TcpStream)?;
        stream
            .set_read_timeout(self.read_timeout)
            .map_err(Error::TcpStream)?;
        stream
            .set_write_timeout(self.write_timeout)
            .map_err(Error::TcpStream)?;
        let stream = ssl.connect(&self.url, stream).map_err(Error::SslStream)?;
        let stream = Arc::new(Mutex::new(stream));

        if self.stream.is_none() {
            self.stream = Some(stream);
            Ok(())
        } else {
            Err(Error::AlreadyConnected)
        }
    }

    pub fn set_read_timeout(&mut self, timeout: Option<Duration>) -> Result<(), Error> {
        if let Some(stream) = self.stream.as_mut() {
            let mut stream = stream.lock().map_err(|_| Error::Mutex)?;
            stream
                .get_mut()
                .set_read_timeout(timeout)
                .map_err(Error::TcpStream)?;
        }
        self.read_timeout = timeout;
        Ok(())
    }

    pub fn set_write_timeout(&mut self, timeout: Option<Duration>) -> Result<(), Error> {
        if let Some(stream) = self.stream.as_mut() {
            let mut stream = stream.lock().map_err(|_| Error::Mutex)?;
            stream
                .get_mut()
                .set_write_timeout(timeout)
                .map_err(Error::TcpStream)?;
        }
        self.write_timeout = timeout;
        Ok(())
    }

    pub fn send(stream: &mut ssl::SslStream<net::TcpStream>, request: &str) -> Result<(), Error> {
        stream
            .write_all(request.as_bytes())
            .map_err(Error::TcpStream)?;
        // add a \n char for EOL
        stream.write_all(&[10]).map_err(Error::TcpStream)?;
        stream.flush().map_err(Error::TcpStream)?;
        Ok(())
    }

    fn raw_read(
        stream: &mut ssl::SslStream<net::TcpStream>,
        blocking: bool,
    ) -> Result<Option<String>, Error> {
        let mut peek_buffer = [0u8; PEEK_BUFFER_SIZE];
        // SslStream will block if `nonblocking` is false
        stream
            .get_mut()
            .set_nonblocking(true)
            .map_err(|_| Error::SetNonBlocking)?;
        // SslStream.ssl_peek() will error if there is no data in the
        // stream receiving end
        let peek = stream.ssl_peek(&mut peek_buffer).ok();
        stream
            .get_mut()
            .set_nonblocking(false)
            .map_err(|_| Error::SetBlocking)?;

        // If blocking or data in the receiving end of the stream
        if blocking || peek.is_some() {
            let mut response = String::new();
            let mut reader = BufReader::new(stream);
            reader.read_line(&mut response).map_err(Error::TcpStream)?;
            Ok(Some(response))
        } else {
            Ok(None)
        }
    }

    pub fn try_read(stream: &mut ssl::SslStream<net::TcpStream>) -> Result<Option<String>, Error> {
        Self::raw_read(stream, false)
    }

    pub fn read(stream: &mut ssl::SslStream<net::TcpStream>) -> Result<String, Error> {
        Ok(Self::raw_read(stream, true)?.expect("blocking"))
    }

    pub fn close(&mut self) -> Result<(), Error> {
        if let Some(stream) = self.stream.take() {
            stream
                .try_lock()
                .map_err(|_| Error::Mutex)?
                .shutdown()
                .map_err(|_| Error::ShutDown)?;
            Ok(())
        } else {
            Err(Error::NotConnected)
        }
    }
}
