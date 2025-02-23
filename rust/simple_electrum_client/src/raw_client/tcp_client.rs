use super::{Error, PEEK_BUFFER_SIZE};
use std::{
    io::{BufRead, BufReader, Write},
    net,
    sync::{Arc, Mutex},
    time::Duration,
};

type TcpStream = Arc<Mutex<net::TcpStream>>;

#[derive(Debug)]
pub struct TcpClient {
    url: String,
    port: u16,
    pub(crate) stream: Option<TcpStream>,
    pub(crate) read_timeout: Option<Duration>,
    pub(crate) write_timeout: Option<Duration>,
}

impl Clone for TcpClient {
    fn clone(&self) -> Self {
        Self {
            url: self.url.clone(),
            port: self.port,
            stream: self.stream.clone(),
            read_timeout: self.read_timeout,
            write_timeout: self.write_timeout,
        }
    }
}

#[allow(clippy::derivable_impls)]
impl Default for TcpClient {
    fn default() -> Self {
        Self {
            url: Default::default(),
            port: 50002,
            stream: None,
            read_timeout: None,
            write_timeout: None,
        }
    }
}

impl Drop for TcpClient {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

impl TcpClient {
    pub fn url(mut self, url: &str) -> Self {
        if !self.is_connected() {
            self.url = url.into();
        } else {
            log::error!("Cannot change url of a connected TcpClient!")
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
        let stream = net::TcpStream::connect(url).map_err(Error::TcpStream)?;
        stream
            .set_read_timeout(self.read_timeout)
            .map_err(Error::TcpStream)?;
        stream
            .set_write_timeout(self.write_timeout)
            .map_err(Error::TcpStream)?;
        if self.stream.is_none() {
            self.stream = Some(Arc::new(Mutex::new(stream)));
            Ok(())
        } else {
            Err(Error::AlreadyConnected)
        }
    }

    pub fn send(stream: &mut net::TcpStream, request: &str) -> Result<(), Error> {
        stream
            .write_all(request.as_bytes())
            .map_err(Error::TcpStream)?;
        // add a \n char for EOL
        stream.write_all(&[10]).map_err(Error::TcpStream)?;
        stream.flush().map_err(Error::TcpStream)?;
        Ok(())
    }

    pub fn set_read_timeout(&mut self, timeout: Option<Duration>) -> Result<(), Error> {
        if let Some(stream) = self.stream.as_mut() {
            let stream = stream.lock().map_err(|_| Error::Mutex)?;
            stream.set_read_timeout(timeout).map_err(Error::TcpStream)?;
        }
        self.read_timeout = timeout;
        Ok(())
    }

    pub fn set_write_timeout(&mut self, timeout: Option<Duration>) -> Result<(), Error> {
        if let Some(stream) = self.stream.as_mut() {
            let stream = stream.lock().map_err(|_| Error::Mutex)?;
            stream
                .set_write_timeout(timeout)
                .map_err(Error::TcpStream)?;
        }
        self.write_timeout = timeout;
        Ok(())
    }

    fn raw_read(stream: &mut net::TcpStream, blocking: bool) -> Result<Option<String>, Error> {
        let mut peek_buffer = [0u8; PEEK_BUFFER_SIZE];

        // TcpStream.peek() if `nonblocking` is false
        stream
            .set_nonblocking(true)
            .map_err(|_| Error::SetNonBlocking)?;
        // If no data in the TcpStream receiving end, TcpStream.peek() will error
        let peek = stream.peek(&mut peek_buffer).ok();
        stream
            .set_nonblocking(false)
            .map_err(|_| Error::SetBlocking)?;

        // If blocking or data in the TcpStream receiving end
        if blocking || peek.is_some() {
            let mut response = String::new();
            let mut reader = BufReader::new(stream.try_clone().map_err(Error::TcpStream)?);
            reader.read_line(&mut response).map_err(Error::TcpStream)?;
            Ok(Some(response))
        } else {
            Ok(None)
        }
    }

    pub fn try_read(stream: &mut net::TcpStream) -> Result<Option<String>, Error> {
        Self::raw_read(stream, false)
    }

    pub fn read(stream: &mut net::TcpStream) -> Result<String, Error> {
        Ok(Self::raw_read(stream, true)?.expect("blocking"))
    }

    pub fn close(&mut self) -> Result<(), Error> {
        if let Some(stream) = self.stream.take() {
            stream
                .try_lock()
                .map_err(|_| Error::Mutex)?
                .shutdown(net::Shutdown::Both)
                .map_err(|_| Error::ShutDown)?;
            Ok(())
        } else {
            Err(Error::NotConnected)
        }
    }
}
