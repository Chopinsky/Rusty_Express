#![allow(dead_code)]

use std::io::{self, prelude::*, Error, ErrorKind};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::time::Duration;

use crate::native_tls::TlsStream;

pub(crate) enum Stream {
    Tcp(TcpStream),
    Tls(Box<TlsStream<TcpStream>>),
}

impl Stream {
    pub(crate) fn shutdown(&mut self, how: Shutdown) -> io::Result<()> {
        match self {
            Stream::Tcp(tcp) => tcp.shutdown(how),
            Stream::Tls(ref mut tls) => tls.shutdown(),
        }
    }

    pub(crate) fn set_read_timeout(&self, dur: Option<Duration>) -> io::Result<()> {
        match self {
            Stream::Tcp(tcp) => tcp.set_read_timeout(dur),
            Stream::Tls(tls) => tls.get_ref().set_read_timeout(dur),
        }
    }

    pub(crate) fn set_write_timeout(&self, dur: Option<Duration>) -> io::Result<()> {
        match self {
            Stream::Tcp(tcp) => tcp.set_write_timeout(dur),
            Stream::Tls(tls) => tls.get_ref().set_write_timeout(dur),
        }
    }

    pub(crate) fn take_error(&self) -> io::Result<Option<io::Error>> {
        match self {
            Stream::Tcp(tcp) => tcp.take_error(),
            Stream::Tls(tls) => tls.get_ref().take_error(),
        }
    }

    pub(crate) fn peer_addr(&self) -> io::Result<SocketAddr> {
        match self {
            Stream::Tcp(tcp) => tcp.peer_addr(),
            Stream::Tls(tls) => tls.get_ref().peer_addr(),
        }
    }

    // Side effect: TLS stream will be downgraded to TCP stream since the handshake has been done
    pub(crate) fn try_clone(&self) -> io::Result<Stream> {
        match self {
            Stream::Tcp(tcp) => tcp.try_clone().map(Stream::Tcp),
            _ => Err(Error::new(
                ErrorKind::InvalidInput,
                "TLS connection shouldn't be kept long-live",
            )),
        }
    }

    pub(crate) fn is_tls(&self) -> bool {
        match self {
            Stream::Tls(_) => true,
            _ => false,
        }
    }
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Stream::Tcp(tcp) => tcp.read(buf),
            Stream::Tls(tls) => tls.read(buf),
        }
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Stream::Tcp(tcp) => tcp.write(buf),
            Stream::Tls(tls) => tls.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Stream::Tcp(tcp) => tcp.flush(),
            Stream::Tls(tls) => tls.flush(),
        }
    }
}
