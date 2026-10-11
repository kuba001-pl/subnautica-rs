//! Where datagrams go: a real UDP socket, or [`crate::sim`]'s network.

use std::io;
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};

/// Sends and receives whole datagrams, never blocking.
pub trait Transport {
    fn send(&mut self, to: SocketAddr, data: &[u8]) -> io::Result<()>;
    /// The next datagram, if one is waiting: its length in `buf` and its
    /// sender.
    fn recv(&mut self, buf: &mut [u8]) -> io::Result<Option<(usize, SocketAddr)>>;
    fn local_addr(&self) -> io::Result<SocketAddr>;
}

/// A non-blocking UDP socket.
pub struct UdpTransport {
    socket: UdpSocket,
}

impl UdpTransport {
    /// Binds to `addr` (port 0: any free port).
    pub fn bind(addr: impl ToSocketAddrs) -> io::Result<UdpTransport> {
        let socket = UdpSocket::bind(addr)?;
        socket.set_nonblocking(true)?;
        Ok(UdpTransport { socket })
    }
}

impl Transport for UdpTransport {
    fn send(&mut self, to: SocketAddr, data: &[u8]) -> io::Result<()> {
        match self.socket.send_to(data, to) {
            Ok(_) => Ok(()),
            // A full send buffer drops the datagram, as the network may.
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(()),
            Err(e) => Err(e),
        }
    }

    fn recv(&mut self, buf: &mut [u8]) -> io::Result<Option<(usize, SocketAddr)>> {
        loop {
            match self.socket.recv_from(buf) {
                Ok((n, from)) => return Ok(Some((n, from))),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(None),
                // Windows reports an earlier datagram's ICMP "port
                // unreachable" on the next receive; it is not about this
                // one.
                Err(e) if e.kind() == io::ErrorKind::ConnectionReset => continue,
                Err(e) => return Err(e),
            }
        }
    }

    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }
}
