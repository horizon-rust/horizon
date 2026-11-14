//! A blocking connection with vanilla's pipeline: optional AES/CFB8
//! encryption, VarInt21 framing and threshold compression. After login it
//! splits into a reader (for a receive thread) and a shareable writer, or
//! gives its stream up ([`Connection::detach`]) to be polled without blocking,
//! the halves then keeping only the framing, compression and encryption. It
//! runs over TCP, or over an in-memory pipe between two threads of one process
//! (vanilla's `LocalChannel`, which an integrated server's player uses).
use crate::{Error, Result, buf::Reader, crypto::Cfb8, frame};
use std::{
    io::{Read, Write},
    net::{Shutdown, SocketAddr, TcpStream, ToSocketAddrs},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicI64, Ordering},
    },
    time::{Duration, Instant},
};

/// Called when a [`Pipe`] end has bytes to read or was closed ([`Pipe::on_ready`]).
type Notify = Arc<dyn Fn() + Send + Sync>;

/// One direction of a [`Pipe`]: the bytes written and not read yet, and
/// whether either end closed it.
#[derive(Default)]
struct Channel {
    state: Mutex<(std::collections::VecDeque<u8>, bool)>,
    ready: Condvar,
    /// The reading end's [`Pipe::on_ready`].
    notify: Mutex<Option<Notify>>,
}
impl Channel {
    fn close(&self) {
        self.state.lock().unwrap().1 = true;
        self.ready.notify_all();
        self.notify();
    }
    /// Tell a reader that doesn't block, outside the state's lock.
    fn notify(&self) {
        let notify = self.notify.lock().unwrap().clone();
        if let Some(notify) = notify {
            notify();
        }
    }
}
/// The channels of one end; dropping the end's last clone closes them, as
/// a socket closes with its process.
struct End {
    incoming: Arc<Channel>,
    outgoing: Arc<Channel>,
}
impl Drop for End {
    fn drop(&mut self) {
        self.incoming.close();
        self.outgoing.close();
    }
}
/// One end of an in-memory byte pipe: what one end writes the other reads.
pub struct Pipe {
    end: Arc<End>,
    /// The read timeout (`set_read_timeout`).
    timeout: Mutex<Option<Duration>>,
    /// Reads with nothing to read fail with `WouldBlock` instead of waiting.
    nonblocking: bool,
}
impl Pipe {
    /// Two connected ends.
    pub fn pair() -> (Self, Self) {
        let (a, b) = (Arc::new(Channel::default()), Arc::new(Channel::default()));
        let end = |incoming, outgoing| Self {
            end: Arc::new(End { incoming, outgoing }),
            timeout: Mutex::new(None),
            nonblocking: false,
        };
        (end(a.clone(), b.clone()), end(b, a))
    }
    fn try_clone(&self) -> Self {
        Self {
            end: self.end.clone(),
            timeout: Mutex::new(*self.timeout.lock().unwrap()),
            nonblocking: self.nonblocking,
        }
    }
    /// Reads with nothing to read fail with `WouldBlock`, as a non-blocking socket's do.
    pub fn set_nonblocking(&mut self, nonblocking: bool) {
        self.nonblocking = nonblocking;
    }
    /// Call `notify` whenever bytes arrive at this end or the pipe closes (also
    /// now, if either already happened), as a poller's readiness would: a
    /// non-blocking reader then reads until `WouldBlock`.
    pub fn on_ready(&self, notify: impl Fn() + Send + Sync + 'static) {
        let incoming = &self.end.incoming;
        *incoming.notify.lock().unwrap() = Some(Arc::new(notify));
        let state = incoming.state.lock().unwrap();
        let ready = !state.0.is_empty() || state.1;
        drop(state);
        if ready {
            incoming.notify();
        }
    }
    /// Close both directions: reads at either end then see the end of the stream.
    pub fn shutdown(&self) {
        self.end.incoming.close();
        self.end.outgoing.close();
    }
}
impl Read for Pipe {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        let deadline = self.timeout.lock().unwrap().map(|t| Instant::now() + t);
        let mut state = self.end.incoming.state.lock().unwrap();
        if self.nonblocking && state.0.is_empty() && !state.1 {
            return Err(std::io::ErrorKind::WouldBlock.into());
        }
        while state.0.is_empty() && !state.1 {
            state = match deadline {
                None => self.end.incoming.ready.wait(state).unwrap(),
                Some(deadline) => {
                    let left = deadline.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        return Err(std::io::ErrorKind::TimedOut.into());
                    }
                    self.end.incoming.ready.wait_timeout(state, left).unwrap().0
                }
            };
        }
        state.0.read(out)
    }
}
impl Write for Pipe {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        let mut state = self.end.outgoing.state.lock().unwrap();
        if state.1 {
            return Err(std::io::ErrorKind::BrokenPipe.into());
        }
        state.0.extend(data);
        self.end.outgoing.ready.notify_all();
        drop(state);
        self.end.outgoing.notify();
        Ok(data.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// What a connection runs over.
enum Stream {
    Tcp(TcpStream),
    Memory(Pipe),
    /// Given up by [`Connection::detach`]: reads and writes fail.
    Detached,
}
fn detached() -> std::io::Error {
    std::io::ErrorKind::NotConnected.into()
}
impl Stream {
    fn try_clone(&self) -> std::io::Result<Self> {
        Ok(match self {
            Self::Tcp(s) => Self::Tcp(s.try_clone()?),
            Self::Memory(p) => Self::Memory(p.try_clone()),
            Self::Detached => Self::Detached,
        })
    }
    fn set_read_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        match self {
            Self::Tcp(s) => s.set_read_timeout(timeout),
            Self::Memory(p) => {
                *p.timeout.lock().unwrap() = timeout;
                Ok(())
            }
            Self::Detached => Ok(()),
        }
    }
    fn shutdown(&self) {
        match self {
            Self::Tcp(s) => {
                let _ = s.shutdown(Shutdown::Both);
            }
            Self::Memory(p) => p.shutdown(),
            Self::Detached => {}
        }
    }
}
impl Read for Stream {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Tcp(s) => s.read(out),
            Self::Memory(p) => p.read(out),
            Self::Detached => Err(detached()),
        }
    }
}
impl Write for Stream {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        match self {
            Self::Tcp(s) => s.write(data),
            Self::Memory(p) => p.write(data),
            Self::Detached => Err(detached()),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Self::Tcp(s) => s.flush(),
            Self::Memory(p) => p.flush(),
            Self::Detached => Err(detached()),
        }
    }
}

/// A connection's stream, given up by [`Connection::detach`].
pub enum Socket {
    Tcp(TcpStream),
    Memory(Pipe),
}

/// Packets received/sent and bytes in/out, for the debug screen.
#[derive(Debug, Default)]
pub struct Counters {
    pub received: AtomicI64,
    pub sent: AtomicI64,
    pub bytes_in: AtomicI64,
    pub bytes_out: AtomicI64,
}

pub struct ConnReader {
    stream: Stream,
    /// Bytes read: `buffer[start..end]` are not consumed yet, and reads go
    /// straight into `buffer[end..]`.
    buffer: Vec<u8>,
    start: usize,
    end: usize,
    inflater: frame::Inflater,
    decrypt: Option<Cfb8>,
    pub threshold: Option<usize>,
    pub counters: Arc<Counters>,
    /// Set for a server of an older release: what arrives is renumbered.
    adapter: Option<Arc<crate::adapter::Session>>,
    /// Set on a server for a client of an older release: what arrives is renumbered.
    older_client: Option<Arc<crate::adapter::ClientSession>>,
}
/// Closes a connection from another thread: a read or a write blocked on it fails.
pub struct Closer(Stream);
impl Closer {
    pub fn close(&self) {
        self.0.shutdown();
    }
}

pub struct ConnWriter {
    stream: Stream,
    encrypt: Option<Cfb8>,
    pub threshold: Option<usize>,
    counters: Arc<Counters>,
    /// Set for a server of an older release: what is sent is renumbered.
    adapter: Option<Arc<crate::adapter::Session>>,
    /// Set on a server for a client of an older release: what is sent is renumbered.
    older_client: Option<Arc<crate::adapter::ClientSession>>,
}
/// `InetAddress.getByName` under Java's default `java.net.preferIPv6Addresses=false`: IPv4
/// addresses first, whatever order the system resolver gives (macOS lists IPv6 first). Vanilla
/// connects to that first address only; the rest stay here as fallbacks.
fn ipv4_first(mut addrs: Vec<SocketAddr>) -> Vec<SocketAddr> {
    addrs.sort_by_key(SocketAddr::is_ipv6);
    addrs
}
/// RFC 8305's connection attempt delay: how long an address has alone before the next
/// one is tried beside it.
const ATTEMPT_DELAY: Duration = Duration::from_millis(250);
/// Both halves before the split.
pub struct Connection {
    pub reader: ConnReader,
    pub writer: ConnWriter,
}
fn configure_stream(stream: &TcpStream) -> Result<()> {
    stream.set_nodelay(true)?;
    let sock = socket2::SockRef::from(stream);
    let keepalive = socket2::TcpKeepalive::new()
        .with_time(Duration::from_secs(10))
        .with_interval(Duration::from_secs(3));
    #[cfg(not(any(target_os = "windows", target_os = "openbsd")))]
    let keepalive = keepalive.with_retries(3);
    let _ = sock.set_tcp_keepalive(&keepalive);
    #[cfg(target_os = "linux")]
    {
        let _ = sock.set_quickack(true);
    }
    Ok(())
}

impl Connection {
    pub fn connect(host: &str, port: u16, timeout: Duration) -> Result<Self> {
        let addrs: Vec<_> = (host, port)
            .to_socket_addrs()
            .map_err(|e| Error::new(format!("Unknown host {host}: {e}")))?
            .collect();
        Self::connect_addresses(host, addrs, timeout)
    }
    /// Connect through Minecraft's SRV redirect. Status requests still use the
    /// original address in their handshake (`ServerStatusPinger.pingServer`).
    pub fn connect_server(host: &str, port: u16, timeout: Duration) -> Result<Self> {
        Self::connect_server_with_address(host, port, timeout).map(|(connection, _, _)| connection)
    }
    /// `ConnectScreen.startConnecting`: login handshakes use the resolved host
    /// and port, including an SRV redirect, rather than the status request's address.
    pub fn connect_server_with_address(
        host: &str,
        port: u16,
        timeout: Duration,
    ) -> Result<(Self, String, u16)> {
        let resolved = crate::resolver::resolve(host, port)?;
        let connection = Self::connect_addresses(host, resolved.addresses, timeout)?;
        Ok((connection, resolved.host, resolved.port))
    }
    /// Each address starts once the earlier ones fail or [`ATTEMPT_DELAY`] after the last
    /// start, alongside those still trying, and the first to connect wins: a dead address
    /// in a round-robin set costs 250 ms instead of the whole timeout.
    fn connect_addresses(host: &str, addrs: Vec<SocketAddr>, timeout: Duration) -> Result<Self> {
        Self::connect_addresses_with(host, addrs, timeout, |addr, timeout| {
            TcpStream::connect_timeout(&addr, timeout)
        })
    }
    fn connect_addresses_with<F>(
        host: &str,
        addrs: Vec<SocketAddr>,
        timeout: Duration,
        connect: F,
    ) -> Result<Self>
    where
        F: Fn(SocketAddr, Duration) -> std::io::Result<TcpStream> + Clone + Send + 'static,
    {
        let addrs = ipv4_first(addrs);
        let (send, recv) = std::sync::mpsc::channel();
        let mut errors: Vec<Option<std::io::Error>> = addrs.iter().map(|_| None).collect();
        let (mut started, mut pending) = (0, 0);
        loop {
            if let Some(&addr) = addrs.get(started) {
                let (send, index) = (send.clone(), started);
                let connect = connect.clone();
                let spawned = std::thread::Builder::new()
                    .name("horizon-connect".into())
                    .spawn(move || {
                        let _ = send.send((index, connect(addr, timeout)));
                    });
                match spawned {
                    Ok(_) => pending += 1,
                    Err(e) => errors[index] = Some(e),
                }
                started += 1;
            }
            if pending == 0 {
                if started < addrs.len() {
                    continue;
                }
                break;
            }
            let reply = if started < addrs.len() {
                match recv.recv_timeout(ATTEMPT_DELAY) {
                    Ok(reply) => reply,
                    Err(_) => continue,
                }
            } else {
                recv.recv().expect("a sender is held here")
            };
            pending -= 1;
            match reply {
                (_, Ok(stream)) => return Self::from_stream(stream),
                (index, Err(e)) => errors[index] = Some(e),
            }
        }
        // Report the first address's failure: it is the one vanilla tries, and the IPv6
        // fallbacks after it often fail only because the client has no IPv6 route.
        match errors.into_iter().flatten().next() {
            Some(e) => Err(Error::new(format!("Connection refused: {e}"))),
            None => Err(Error::new(format!("Unknown host {host}"))),
        }
    }
    pub fn from_stream(stream: TcpStream) -> Result<Self> {
        configure_stream(&stream)?;
        Self::over(Stream::Tcp(stream))
    }
    /// A connection over one end of an in-memory [`Pipe`].
    pub fn local(pipe: Pipe) -> Self {
        Self::over(Stream::Memory(pipe)).expect("a pipe end clones")
    }
    fn over(stream: Stream) -> Result<Self> {
        let counters = Arc::new(Counters::default());
        let write = stream.try_clone()?;
        Ok(Self {
            reader: ConnReader {
                stream,
                buffer: Vec::new(),
                start: 0,
                end: 0,
                inflater: frame::Inflater::default(),
                decrypt: None,
                threshold: None,
                counters: counters.clone(),
                adapter: None,
                older_client: None,
            },
            writer: ConnWriter {
                stream: write,
                encrypt: None,
                threshold: None,
                counters,
                adapter: None,
                older_client: None,
            },
        })
    }
    /// Speak to a server of `version` from now on (before the handshake): packets are renumbered both
    /// ways (crate::adapter). The pinned version needs nothing.
    pub fn adapt(&mut self, version: crate::adapter::ServerVersion) {
        let session = (version != crate::adapter::ServerVersion::Current)
            .then(|| Arc::new(crate::adapter::Session::new(version)));
        self.reader.adapter = session.clone();
        self.writer.adapter = session;
    }
    /// Serve a client of `version` from now on (after its handshake): packets are renumbered both ways
    /// (crate::adapter::ClientSession). The pinned version needs nothing.
    pub fn serve_older(&mut self, version: crate::adapter::ServerVersion) {
        let session = (version != crate::adapter::ServerVersion::Current)
            .then(|| Arc::new(crate::adapter::ClientSession::new(version)));
        self.reader.older_client = session.clone();
        self.writer.older_client = session;
    }
    /// The older release this connection's client is on ([`Self::serve_older`]), `None` for the pinned one.
    pub fn older_client(&self) -> Option<crate::adapter::ServerVersion> {
        self.reader.older_client.as_ref().map(|s| s.version)
    }
    /// `Connection.isMemoryConnection`: through a [`Pipe`] rather than a socket.
    pub fn is_memory(&self) -> bool {
        matches!(self.reader.stream, Stream::Memory(_))
    }
    /// ClientboundLoginCompressionPacket: negative thresholds disable compression.
    pub fn set_compression(&mut self, threshold: i32) {
        let t = usize::try_from(threshold).ok();
        self.reader.threshold = t;
        self.writer.threshold = t;
    }
    /// Connection.setEncryptionKey: both directions keyed with the shared secret.
    pub fn enable_encryption(&mut self, secret: &[u8; 16]) {
        let mut decrypt = Cfb8::new(secret, secret);
        // Bytes already read past the last packet were sent encrypted.
        let r = &mut self.reader;
        decrypt.decrypt(&mut r.buffer[r.start..r.end]);
        self.reader.decrypt = Some(decrypt);
        self.writer.encrypt = Some(Cfb8::new(secret, secret));
    }
    pub fn send(&mut self, body: &[u8]) -> Result<()> {
        self.writer.send(body)
    }
    pub fn recv(&mut self) -> Result<(i32, Vec<u8>)> {
        self.reader.recv()
    }
    pub fn split(self) -> (ConnReader, ConnWriter) {
        (self.reader, self.writer)
    }
    /// Take the stream out, to read and write it elsewhere (a poller's thread,
    /// without blocking): the halves keep the framing, compression and
    /// encryption, and bytes already read past the last packet. What arrives
    /// goes in through [`ConnReader::read_from`]; [`ConnWriter::frame_into`]
    /// makes what is to be sent. Their own reads and writes fail from now on.
    pub fn detach(mut self) -> (Socket, ConnReader, ConnWriter) {
        self.writer.stream = Stream::Detached;
        let socket = match std::mem::replace(&mut self.reader.stream, Stream::Detached) {
            Stream::Tcp(s) => Socket::Tcp(s),
            Stream::Memory(p) => Socket::Memory(p),
            Stream::Detached => unreachable!("a connection is detached once, by value"),
        };
        (socket, self.reader, self.writer)
    }
}

/// A packet read off a frame: (id, payload, the frame's contents when it was
/// compressed and they were asked for).
type Packet = (i32, Vec<u8>, Option<Vec<u8>>);

/// A packet with the frame it came in ([`ConnReader::recv_framed`]): a proxy
/// whose two sides use the same threshold can send `compressed` on as it is
/// ([`ConnWriter::send_contents`]) instead of compressing `payload` again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Framed {
    pub id: i32,
    pub payload: Vec<u8>,
    /// The frame's contents as they arrived (the data length, then the zlib
    /// stream) when it was compressed, copied for the caller; None when it was
    /// not, or when a version adapter or an older client's renumbering changed
    /// the packet.
    pub compressed: Option<Vec<u8>>,
}
impl From<Packet> for Framed {
    fn from((id, payload, compressed): Packet) -> Self {
        Self {
            id,
            payload,
            compressed,
        }
    }
}

/// The next frame in a reader's buffer.
enum Next {
    Packet(Packet),
    /// A frame with no packet in it.
    Empty,
    /// No whole frame yet: the bytes it needs, once its length is known (0 before).
    Partial(usize),
}
impl ConnReader {
    /// Read exactly `len` bytes as they are, with no framing.
    pub fn read_raw(&mut self, len: usize) -> Result<Vec<u8>> {
        let mut out = vec![0u8; len];
        self.stream.read_exact(&mut out)?;
        Ok(out)
    }
    /// Block until a whole packet arrives: (id, payload).
    pub fn recv(&mut self) -> Result<(i32, Vec<u8>)> {
        self.recv_with(false).map(|(id, payload, _)| (id, payload))
    }
    /// [`Self::recv`], with the frame: read, inflated and checked as `recv`
    /// does, and counted the same.
    pub fn recv_framed(&mut self) -> Result<Framed> {
        self.recv_with(true).map(Framed::from)
    }
    /// [`Self::buffered`], with the frame.
    pub fn buffered_framed(&mut self) -> Result<Option<Framed>> {
        Ok(self.buffered_with(true)?.map(Framed::from))
    }
    /// [`Self::recv`], keeping a compressed frame's contents when `keep_frame`
    /// is set and no adapter renumbers what arrives.
    fn recv_with(&mut self, keep_frame: bool) -> Result<Packet> {
        if let Some((id, payload)) = self.adapter.as_ref().and_then(|a| a.next_incoming()) {
            return Ok((id, payload, None));
        }
        let keep_frame = keep_frame && self.adapter.is_none() && self.older_client.is_none();
        loop {
            match self.next(keep_frame)? {
                Next::Packet((id, payload, _)) if self.older_client.is_some() => {
                    let client = self.older_client.as_ref().expect("checked");
                    if let Some((id, payload)) = client.serverbound(id, payload)? {
                        return Ok((id, payload, None));
                    }
                }
                Next::Packet(packet) => match &self.adapter {
                    None => return Ok(packet),
                    Some(a) => {
                        if let Some((id, payload)) = a.clientbound(packet.0, packet.1)? {
                            return Ok((id, payload, None));
                        }
                        if let Some((id, payload)) = a.next_incoming() {
                            return Ok((id, payload, None));
                        }
                    }
                },
                Next::Empty => {}
                Next::Partial(wanted) => self.fill(wanted)?,
            }
        }
    }
    /// The next whole packet among the bytes already read, without reading
    /// more: for a detached reader, after [`Self::read_from`]. Only an older
    /// client's renumbering applies (a server's connection has no other).
    pub fn buffered(&mut self) -> Result<Option<(i32, Vec<u8>)>> {
        Ok(self
            .buffered_with(false)?
            .map(|(id, payload, _)| (id, payload)))
    }
    /// [`Self::buffered`], keeping a compressed frame's contents when
    /// `keep_frame` is set and no older client's renumbering applies.
    fn buffered_with(&mut self, keep_frame: bool) -> Result<Option<Packet>> {
        let keep_frame = keep_frame && self.older_client.is_none();
        loop {
            match self.next(keep_frame)? {
                Next::Packet((id, payload, _)) if self.older_client.is_some() => {
                    let client = self.older_client.as_ref().expect("checked");
                    if let Some((id, payload)) = client.serverbound(id, payload)? {
                        return Ok(Some((id, payload, None)));
                    }
                }
                Next::Packet(packet) => return Ok(Some(packet)),
                Next::Empty => {}
                Next::Partial(_) => return Ok(None),
            }
        }
    }
    /// Read once from `source` (a detached stream, without blocking) into the
    /// buffer: the bytes read, 0 at the end of the stream, or its error
    /// (`WouldBlock` when nothing waits).
    pub fn read_from(&mut self, source: &mut impl Read) -> std::io::Result<usize> {
        let wanted = match self.next_header() {
            Ok(Some((prefix, len))) => prefix + len,
            _ => 0,
        };
        self.fill_from(source, wanted)
    }
    /// The completed next frame header: (length-prefix bytes, payload bytes).
    fn next_header(&self) -> Result<Option<(usize, usize)>> {
        frame::header(&self.buffer[self.start..self.end])
    }
    /// Take the next frame off the buffer, if it is whole; with `keep_frame`, a
    /// compressed frame's contents come out with its packet.
    fn next(&mut self, keep_frame: bool) -> Result<Next> {
        // Read the VarInt21 length once; the same prefix is used to locate
        // the payload and to determine whether the full frame is available.
        let Some((prefix, len)) = self.next_header()? else {
            return Ok(Next::Partial(0));
        };
        let wanted = prefix + len;
        if self.end - self.start < wanted {
            return Ok(Next::Partial(wanted));
        }
        let frame = self.start + prefix..self.start + wanted;
        self.start = frame.end;
        Ok(match self.packet(frame, keep_frame)? {
            Some(packet) => {
                self.counters.received.fetch_add(1, Ordering::Relaxed);
                Next::Packet(packet)
            }
            None => Next::Empty,
        })
    }
    /// The packet in `buffer[frame]`, or `None` for a frame with no packet in it:
    /// vanilla's PacketDecoder only decodes a packet from a readable buffer, so an
    /// empty compressed frame is skipped and the connection stays up. With
    /// `keep_frame`, a compressed frame's contents (the data length, then the
    /// zlib stream) come with the packet, copied.
    fn packet(
        &mut self,
        frame: std::ops::Range<usize>,
        keep_frame: bool,
    ) -> Result<Option<Packet>> {
        let frame = &self.buffer[frame];
        match frame::body(frame, self.threshold, &mut self.inflater)? {
            frame::Body::Plain(at) => {
                let body = &frame[at..];
                if body.is_empty() {
                    return Ok(None);
                }
                let mut r = Reader::new(body);
                let id = r.varint()?;
                Ok(Some((id, body[r.position()..].to_vec(), None)))
            }
            frame::Body::Inflated(mut body) => {
                let mut r = Reader::new(&body);
                let id = r.varint()?;
                let at = r.position();
                body.drain(..at);
                Ok(Some((id, body, keep_frame.then(|| frame.to_vec()))))
            }
        }
    }
    /// Read once from the stream into the buffer, with room for at least
    /// `frame` bytes of unconsumed data (a whole frame, once its length is known).
    fn fill(&mut self, frame: usize) -> Result<()> {
        let mut stream = std::mem::replace(&mut self.stream, Stream::Detached);
        let read = self.fill_from(&mut stream, frame);
        self.stream = stream;
        match read {
            Ok(0) => Err(Error::end_of_stream()),
            Ok(_) => Ok(()),
            // A signal (a profiler attaching, SIGCHLD...) cut the read short
            // before any byte came: nothing is wrong with the connection, the
            // caller reads again.
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => Ok(()),
            // The read timeout passed: Windows reports TimedOut, Unix
            // WouldBlock.
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ) =>
            {
                Err(Error::timed_out())
            }
            Err(e) => Err(e.into()),
        }
    }
    /// [`Self::fill`] from `source`: the bytes read, decrypted into the buffer.
    fn fill_from(&mut self, source: &mut impl Read, frame: usize) -> std::io::Result<usize> {
        const READ: usize = 1 << 16;
        if self.start == self.end {
            (self.start, self.end) = (0, 0);
        }
        let pending = self.end - self.start;
        let room = READ.max(frame.saturating_sub(pending));
        if self.buffer.len() - self.end < room {
            self.buffer.copy_within(self.start..self.end, 0);
            (self.start, self.end) = (0, pending);
            if self.buffer.len() < pending + room {
                self.buffer.resize(pending + room, 0);
            }
        }
        let n = source.read(&mut self.buffer[self.end..])?;
        self.counters
            .bytes_in
            .fetch_add(n as i64, Ordering::Relaxed);
        let data = &mut self.buffer[self.end..self.end + n];
        if let Some(c) = &mut self.decrypt {
            c.decrypt(data);
        }
        self.end += n;
        Ok(n)
    }
    /// A [`Closer`] for this connection.
    pub fn closer(&self) -> Result<Closer> {
        Ok(Closer(self.stream.try_clone()?))
    }
    pub fn set_read_timeout(&self, timeout: Option<Duration>) -> Result<()> {
        Ok(self.stream.set_read_timeout(timeout)?)
    }
}
impl ConnWriter {
    /// The server release selected for this connection (local play uses the pinned release).
    pub fn server_version(&self) -> crate::adapter::ServerVersion {
        self.adapter
            .as_ref()
            .map_or(crate::adapter::ServerVersion::Current, |a| a.version)
    }

    /// The old ServerData flag, once it arrived on a pre-component connection.
    pub fn legacy_secure_chat_enforcement(&self) -> Option<bool> {
        self.adapter
            .as_ref()
            .and_then(|a| a.legacy_secure_chat_enforcement())
    }

    /// Write bytes as they are, with no framing: the legacy server list ping
    /// (`LegacyServerPinger`) is not a framed protocol.
    pub fn write_raw(&mut self, data: &[u8]) -> Result<()> {
        self.stream.write_all(data)?;
        self.stream.flush()?;
        Ok(())
    }
    /// A packet body as this connection's client takes it: renumbered for a client of an older release
    /// ([`Connection::serve_older`]), `None` when it has no such packet; as it is otherwise.
    pub fn for_client<'a>(&self, body: &'a [u8]) -> Result<Option<std::borrow::Cow<'a, [u8]>>> {
        match &self.older_client {
            None => Ok(Some(std::borrow::Cow::Borrowed(body))),
            Some(client) => Ok(client.clientbound(body)?.map(std::borrow::Cow::Owned)),
        }
    }
    /// Send one packet body (VarInt id + payload).
    pub fn send(&mut self, body: &[u8]) -> Result<()> {
        if self.older_client.is_some() {
            return match self.for_client(body)? {
                Some(body) => self.send_contents(&frame::contents(&body, self.threshold)?),
                None => Ok(()),
            };
        }
        if let Some(a) = self.adapter.clone() {
            if let Some(body) = a.serverbound(body)? {
                self.send_contents(&frame::contents(&body, self.threshold)?)?;
            }
            for body in a.take_outgoing() {
                self.send_contents(&frame::contents(&body, self.threshold)?)?;
            }
            return Ok(());
        }
        self.send_contents(&frame::contents(body, self.threshold)?)
    }
    /// Send a frame whose contents [`frame::contents`] made with this
    /// writer's threshold.
    pub fn send_contents(&mut self, contents: &[u8]) -> Result<()> {
        let mut framed = Vec::with_capacity(contents.len() + 3);
        self.frame_into(contents, &mut framed);
        self.stream.write_all(&framed)?;
        Ok(())
    }
    /// The bytes [`Self::send_contents`] writes, appended to `out` instead: a
    /// detached writer's, written later, in the order framed (the cipher
    /// runs on).
    pub fn frame_into(&mut self, contents: &[u8], out: &mut Vec<u8>) {
        let at = out.len();
        crate::buf::write_varint(out, contents.len() as i32);
        out.extend_from_slice(contents);
        if let Some(c) = &mut self.encrypt {
            c.encrypt(&mut out[at..]);
        }
        self.counters.sent.fetch_add(1, Ordering::Relaxed);
        self.counters
            .bytes_out
            .fetch_add((out.len() - at) as i64, Ordering::Relaxed);
    }
    pub fn shutdown(&self) {
        self.stream.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn ipv4_first_keeps_each_familys_order() {
        let a = |s: &str| s.parse::<SocketAddr>().unwrap();
        let resolved = vec![
            a("[2001:db8::1]:25565"),
            a("192.0.2.1:25565"),
            a("[2001:db8::2]:25565"),
            a("10.0.0.1:25565"),
        ];
        assert_eq!(
            ipv4_first(resolved),
            [
                a("192.0.2.1:25565"),
                a("10.0.0.1:25565"),
                a("[2001:db8::1]:25565"),
                a("[2001:db8::2]:25565"),
            ]
        );
    }

    fn peer(c: &Connection) -> SocketAddr {
        match &c.reader.stream {
            Stream::Tcp(s) => s.peer_addr().unwrap(),
            Stream::Memory(_) | Stream::Detached => panic!("not a socket"),
        }
    }

    /// Binds `[::1]`, or `None` where the machine has no IPv6 loopback.
    fn ipv6_loopback() -> Option<TcpListener> {
        let listener = TcpListener::bind("[::1]:0");
        if listener.is_err() {
            eprintln!("skipped: no IPv6 loopback");
        }
        listener.ok()
    }

    #[test]
    fn connects_to_ipv4_although_the_resolver_lists_ipv6_first() {
        // Both listen, as with a server whose IPv6 address accepts and then resets.
        let Some(v6_listener) = ipv6_loopback() else {
            return;
        };
        let v4_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let v6 = v6_listener.local_addr().unwrap();
        let v4 = v4_listener.local_addr().unwrap();
        let c =
            Connection::connect_addresses("test", vec![v6, v4], Duration::from_secs(5)).unwrap();
        assert_eq!(peer(&c), v4);
    }

    #[test]
    fn falls_back_to_ipv6_when_ipv4_refuses() {
        let Some(v6_listener) = ipv6_loopback() else {
            return;
        };
        let v6 = v6_listener.local_addr().unwrap();
        let closed = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        let c = Connection::connect_addresses("test", vec![v6, closed], Duration::from_secs(5))
            .unwrap();
        assert_eq!(peer(&c), v6);
    }

    #[test]
    fn a_dead_address_does_not_hold_up_the_next() {
        // A controlled pending dial cannot be intercepted by a VPN or proxy.
        let held_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let dead = held_listener.local_addr().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let live = listener.local_addr().unwrap();
        let (release, wait) = std::sync::mpsc::channel::<()>();
        let wait = Arc::new(Mutex::new(wait));
        let pending = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let entered = pending.clone();
        let start = Instant::now();
        let c = Connection::connect_addresses_with(
            "test",
            vec![dead, live],
            Duration::from_secs(10),
            move |addr, timeout| {
                if addr == dead {
                    entered.store(true, Ordering::SeqCst);
                    let _ = wait.lock().unwrap().recv_timeout(Duration::from_secs(5));
                    Err(std::io::ErrorKind::TimedOut.into())
                } else {
                    TcpStream::connect_timeout(&addr, timeout)
                }
            },
        )
        .unwrap();
        drop(release);
        assert!(pending.load(Ordering::SeqCst));
        assert_eq!(peer(&c), live);
        assert!(
            start.elapsed() >= ATTEMPT_DELAY && start.elapsed() < Duration::from_secs(5),
            "{:?}",
            start.elapsed()
        );
    }

    #[test]
    fn every_failure_reports_the_first_address() {
        let closed = |_| {
            TcpListener::bind("127.0.0.1:0")
                .unwrap()
                .local_addr()
                .unwrap()
        };
        let error = |host, addrs| {
            Connection::connect_addresses(host, addrs, Duration::from_secs(5))
                .err()
                .unwrap()
                .to_string()
        };
        let refused = error("test", (0..3).map(closed).collect());
        assert!(refused.starts_with("Connection refused"), "{refused}");
        assert_eq!(error("nowhere", Vec::new()), "Unknown host nowhere");
    }

    #[test]
    fn a_silent_peer_times_out_and_a_closed_one_ends_the_stream() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let mut c = Connection::connect("127.0.0.1", port, Duration::from_secs(5)).unwrap();
        let (server, _) = listener.accept().unwrap();
        c.reader
            .set_read_timeout(Some(Duration::from_millis(100)))
            .unwrap();
        let silent = c.recv().unwrap_err();
        assert!(silent.is_timed_out(), "{silent}");
        assert_eq!(silent.to_string(), "Timed out");
        drop(server);
        let closed = c.recv().unwrap_err();
        assert!(closed.is_end_of_stream(), "{closed}");
    }

    #[test]
    fn loopback_with_compression_and_encryption() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut c = Connection::from_stream(stream).unwrap();
            let (id, payload) = c.recv().unwrap();
            assert_eq!((id, payload.as_slice()), (0, &b"plain"[..]));
            c.enable_encryption(&[7; 16]);
            c.set_compression(64);
            let (id, payload) = c.recv().unwrap();
            assert_eq!((id, payload.len()), (5, 1000));
            let mut body = vec![9];
            body.extend_from_slice(b"reply");
            c.send(&body).unwrap();
        });
        let mut c = Connection::connect("127.0.0.1", port, Duration::from_secs(5)).unwrap();
        let mut body = vec![0];
        body.extend_from_slice(b"plain");
        c.send(&body).unwrap();
        c.enable_encryption(&[7; 16]);
        c.set_compression(64);
        let mut big = vec![5];
        big.extend(std::iter::repeat_n(3u8, 1000));
        c.send(&big).unwrap();
        let (id, payload) = c.recv().unwrap();
        assert_eq!((id, payload.as_slice()), (9, &b"reply"[..]));
        server.join().unwrap();
    }

    #[test]
    fn fragmented_plain_frame_header_and_following_packet() {
        let (pipe, _peer) = Pipe::pair();
        let mut connection = Connection::local(pipe);
        let payload: Vec<u8> = (0..300).map(|i| i as u8).collect();
        let mut body = vec![0xac, 0x02]; // Packet id 300, two-byte VarInt.
        body.extend_from_slice(&payload);
        let mut framed = frame::encode(&body, None).unwrap();
        framed.extend(frame::encode(&[7, 9, 8], None).unwrap());

        for chunk in [&framed[..1], &framed[1..2]] {
            let mut input = chunk;
            assert_eq!(
                connection.reader.read_from(&mut input).unwrap(),
                chunk.len()
            );
            assert!(connection.reader.buffered().unwrap().is_none());
        }
        assert_eq!(
            connection.reader.next_header().unwrap(),
            Some((2, body.len()))
        );
        let mut remaining = &framed[2..];
        assert_eq!(
            connection.reader.read_from(&mut remaining).unwrap(),
            framed.len() - 2
        );
        assert_eq!(connection.reader.buffered().unwrap(), Some((300, payload)));
        assert_eq!(connection.reader.buffered().unwrap(), Some((7, vec![9, 8])));
        assert!(connection.reader.buffered().unwrap().is_none());
        assert_eq!(
            connection.reader.counters.received.load(Ordering::Relaxed),
            2
        );
    }

    #[test]
    fn malformed_frame_headers_preserve_read_and_error_behaviour() {
        for (header, message) in [
            (&[0_u8][..], "Frame length cannot be zero"),
            (&[0x80_u8, 0x80, 0x80][..], "length wider than 21 bits"),
        ] {
            let (pipe, _peer) = Pipe::pair();
            let mut connection = Connection::local(pipe);
            let mut input = header;
            assert_eq!(
                connection.reader.read_from(&mut input).unwrap(),
                header.len()
            );
            assert!(input.is_empty());
            assert_eq!(
                connection.reader.next_header().unwrap_err().to_string(),
                message
            );

            // A malformed buffered header must not make read_from stop reading.
            let mut more = &[1_u8][..];
            assert_eq!(connection.reader.read_from(&mut more).unwrap(), 1);
            assert!(more.is_empty());
            assert_eq!(
                connection.reader.buffered().unwrap_err().to_string(),
                message
            );
        }
    }

    /// In-memory decoding only; not a network or whole-server benchmark.
    /// Run with cargo test --release -p horizon-protocol --lib
    /// packet_decoder_in_memory_microbenchmark -- --ignored --nocapture
    #[test]
    #[ignore = "manual release-mode microbenchmark"]
    fn packet_decoder_in_memory_microbenchmark() {
        use std::hint::black_box;
        const RUNS: usize = 4;
        const SAMPLES_PER_RUN: usize = 15;
        const PACKETS_PER_SAMPLE: usize = 10_000;

        for payload_len in [0, 48, 1024, 65_536] {
            let mut body = vec![0_u8]; // Packet id 0.
            body.extend(vec![0x2a; payload_len]);
            let frame = frame::encode(&body, None).unwrap();
            let mut run_medians = Vec::with_capacity(RUNS);

            for _ in 0..RUNS {
                let mut samples = Vec::with_capacity(SAMPLES_PER_RUN);
                for _ in 0..SAMPLES_PER_RUN {
                    let (pipe, _peer) = Pipe::pair();
                    let mut connection = Connection::local(pipe);
                    let started = Instant::now();

                    for _ in 0..PACKETS_PER_SAMPLE {
                        let mut input = frame.as_slice();
                        while !input.is_empty() {
                            assert!(connection.reader.read_from(&mut input).unwrap() > 0);
                        }
                        let (id, decoded) = connection.reader.buffered().unwrap().unwrap();
                        assert_eq!(id, 0);
                        assert_eq!(decoded.len(), payload_len);
                        black_box(decoded);
                    }
                    samples.push(started.elapsed().as_secs_f64() * 1e9 / PACKETS_PER_SAMPLE as f64);
                }
                samples.sort_by(f64::total_cmp);
                run_medians.push(samples[SAMPLES_PER_RUN / 2]);
            }
            run_medians.sort_by(f64::total_cmp);
            let median = (run_medians[1] + run_medians[2]) / 2.0;
            println!(
                "payload={payload_len} bytes: {median:.2} ns/packet (4 runs, 15 samples/run, 10000 packets/sample)"
            );
        }
    }

    #[test]
    fn local_pipe_carries_packets_and_closes() {
        let (a, b) = Pipe::pair();
        let server = std::thread::spawn(move || {
            let mut c = Connection::local(b);
            c.set_compression(64);
            let (id, payload) = c.recv().unwrap();
            assert_eq!((id, payload.len()), (5, 1000));
            c.send(&[9, 1, 2]).unwrap();
            let (_, writer) = c.split();
            writer.shutdown();
        });
        let mut c = Connection::local(a);
        c.set_compression(64);
        let mut big = vec![5];
        big.extend(std::iter::repeat_n(3u8, 1000));
        c.send(&big).unwrap();
        assert_eq!(c.recv().unwrap(), (9, vec![1, 2]));
        assert_eq!(c.reader.counters.sent.load(Ordering::Relaxed), 1);
        assert_eq!(c.reader.counters.received.load(Ordering::Relaxed), 1);
        server.join().unwrap();
        assert!(c.recv().is_err());
    }

    /// Packets from one byte to past the read size, compressed and
    /// encrypted, arrive whole and in order however the reads cut them.
    #[test]
    fn packets_of_every_size_arrive_whole() {
        let (a, b) = Pipe::pair();
        let bodies: Vec<Vec<u8>> = (0..400u32)
            .map(|i| {
                let len = [1, 2, 63, 64, 65, 300, 4000, 70_000, 200_000][i as usize % 9];
                (0..len).map(|j| (j as u32 * 31 + i) as u8 & 0x7f).collect()
            })
            .collect();
        let sent = bodies.clone();
        let server = std::thread::spawn(move || {
            let mut c = Connection::local(b);
            c.set_compression(64);
            c.enable_encryption(&[3; 16]);
            for body in &sent {
                c.send(body).unwrap();
            }
            c
        });
        let mut c = Connection::local(a);
        c.set_compression(64);
        c.enable_encryption(&[3; 16]);
        for body in &bodies {
            let (id, payload) = c.recv().unwrap();
            assert_eq!(id, i32::from(body[0]));
            assert_eq!(payload, body[1..]);
        }
        drop(server.join().unwrap());
    }

    #[test]
    fn dropping_a_pipe_end_closes_it() {
        let (a, b) = Pipe::pair();
        let mut c = Connection::local(a);
        drop(b);
        assert!(c.recv().is_err());
        assert!(c.send(&[1]).is_err());
    }

    #[test]
    fn local_pipe_read_times_out() {
        let (a, _b) = Pipe::pair();
        let mut c = Connection::local(a);
        c.reader
            .set_read_timeout(Some(Duration::from_millis(20)))
            .unwrap();
        assert!(c.recv().is_err());
    }

    /// Encrypted bytes that arrive in the same read as the last plain packet
    /// are decrypted once encryption is enabled.
    #[test]
    fn encrypted_bytes_read_before_enabling_encryption() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (sent, both_sent) = std::sync::mpsc::channel();
        let client = std::thread::spawn(move || {
            let mut c = Connection::connect("127.0.0.1", port, Duration::from_secs(5)).unwrap();
            c.send(&[0, 1, 2]).unwrap();
            c.enable_encryption(&[7; 16]);
            c.send(&[5, 4, 3]).unwrap();
            sent.send(c).unwrap();
        });
        let (stream, _) = listener.accept().unwrap();
        let mut c = Connection::from_stream(stream).unwrap();
        let _keep_open = both_sent.recv().unwrap();
        assert_eq!(c.recv().unwrap(), (0, vec![1, 2]));
        c.enable_encryption(&[7; 16]);
        assert_eq!(c.recv().unwrap(), (5, vec![4, 3]));
        client.join().unwrap();
    }

    /// A server may send a compressed frame that carries no packet: the
    /// uncompressed length is 0 and the frame holds nothing else. Vanilla
    /// skips it (its PacketDecoder only reads a packet from a readable
    /// buffer), so the client stays connected.
    #[test]
    fn an_empty_compressed_frame_is_skipped() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut c = Connection::from_stream(stream).unwrap();
            c.set_compression(64);
            // The empty frame: its length prefix, then the uncompressed
            // length 0 and nothing else.
            c.writer.send_contents(&[0]).unwrap();
            let mut body = vec![3];
            body.extend_from_slice(b"after");
            c.writer.send(&body).unwrap();
        });
        let mut c = Connection::connect("127.0.0.1", port, Duration::from_secs(5)).unwrap();
        c.set_compression(64);
        let (id, payload) = c.recv().unwrap();
        assert_eq!((id, payload.as_slice()), (3, &b"after"[..]));
        server.join().unwrap();
    }

    /// A body of `len` bytes (its id first) that does not compress to nothing.
    fn body_of(id: u8, len: usize) -> Vec<u8> {
        let mut body = vec![id];
        body.extend((1..len).map(|i| (i * 31 % 251) as u8));
        body
    }

    /// The frame a framed read gives is, byte for byte, the frame's contents
    /// the sender wrote, for bodies around the threshold and far past it, with
    /// encryption on; a body under the threshold has none.
    #[test]
    fn a_framed_read_gives_the_frame_as_it_was_sent() {
        let (a, b) = Pipe::pair();
        let lens = [1, 2, 63, 64, 65, 300, 4000, 70_000, 200_000];
        let mut sender = Connection::local(b);
        sender.set_compression(64);
        sender.enable_encryption(&[5; 16]);
        let mut c = Connection::local(a);
        c.set_compression(64);
        c.enable_encryption(&[5; 16]);
        for (i, &len) in lens.iter().enumerate() {
            let body = body_of(i as u8 + 1, len);
            let sent = frame::contents(&body, Some(64)).unwrap();
            sender.writer.send_contents(&sent).unwrap();
            let got = c.reader.recv_framed().unwrap();
            assert_eq!((got.id, &got.payload[..]), (i32::from(body[0]), &body[1..]));
            let expected = (len >= 64).then_some(sent);
            assert_eq!(got.compressed, expected, "body of {len} bytes");
        }
        assert_eq!(
            c.reader.counters.received.load(Ordering::Relaxed),
            lens.len() as i64
        );
    }

    /// With compression off no frame is compressed, so none is given.
    #[test]
    fn no_frame_with_compression_off() {
        let (a, b) = Pipe::pair();
        let mut sender = Connection::local(b);
        let mut c = Connection::local(a);
        for len in [1, 300, 70_000] {
            let body = body_of(4, len);
            sender.send(&body).unwrap();
            let got = c.reader.recv_framed().unwrap();
            assert_eq!(
                got,
                Framed {
                    id: 4,
                    payload: body[1..].to_vec(),
                    compressed: None
                }
            );
        }
    }

    /// Framed and plain reads mixed on one stream that arrives 7 bytes at a
    /// time, compressed and encrypted, with empty frames between the packets:
    /// each read gives the next packet, the framed ones with their frame, and
    /// the empty frames are skipped and not counted.
    #[test]
    fn framed_and_plain_reads_mix_on_a_stream_cut_in_pieces() {
        let (a, b) = Pipe::pair();
        let mut sender = Connection::local(b);
        sender.set_compression(64);
        sender.enable_encryption(&[9; 16]);
        let bodies: Vec<Vec<u8>> = [3, 70, 64, 500, 1, 63, 9000, 65]
            .iter()
            .enumerate()
            .map(|(i, &len)| body_of(i as u8 + 1, len))
            .collect();
        let mut stream = Vec::new();
        let mut sent = Vec::new();
        for body in &bodies {
            sender.writer.frame_into(&[0], &mut stream);
            let contents = frame::contents(body, Some(64)).unwrap();
            sender.writer.frame_into(&contents, &mut stream);
            sent.push(contents);
        }
        let mut c = Connection::local(a);
        c.set_compression(64);
        c.enable_encryption(&[9; 16]);
        let mut got = Vec::new();
        for piece in stream.chunks(7) {
            assert_eq!(c.reader.read_from(&mut &piece[..]).unwrap(), piece.len());
            loop {
                let next = if got.len() % 2 == 0 {
                    c.reader.buffered_framed().unwrap()
                } else {
                    let plain = c.reader.buffered().unwrap();
                    plain.map(|(id, payload)| Framed {
                        id,
                        payload,
                        compressed: None,
                    })
                };
                match next {
                    Some(packet) => got.push(packet),
                    None => break,
                }
            }
        }
        assert_eq!(got.len(), bodies.len());
        for (i, ((packet, body), contents)) in got.iter().zip(&bodies).zip(&sent).enumerate() {
            assert_eq!(
                (packet.id, &packet.payload[..]),
                (i32::from(body[0]), &body[1..])
            );
            let framed = i % 2 == 0 && body.len() >= 64;
            assert_eq!(
                packet.compressed.as_ref(),
                framed.then_some(contents),
                "packet {i}"
            );
        }
        assert_eq!(
            c.reader.counters.received.load(Ordering::Relaxed),
            bodies.len() as i64
        );
    }

    /// A detached reader fed with `read_from`: the frame the caller holds is
    /// its own copy, unchanged while the reader's buffer is moved, grown and
    /// refilled for the next packets.
    #[test]
    fn a_held_frame_outlives_the_buffer_moving() {
        let (a, b) = Pipe::pair();
        let mut sender = Connection::local(b);
        sender.set_compression(64);
        let mut c = Connection::local(a);
        c.set_compression(64);
        let (_socket, mut reader, _writer) = c.detach();
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut noise = |len: usize| -> Vec<u8> {
            (0..len)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    seed as u8
                })
                .collect()
        };
        let first = [vec![1], noise(1000)].concat();
        let second = [vec![2], noise(300_000)].concat();
        let first_sent = frame::contents(&first, Some(64)).unwrap();
        let mut stream = Vec::new();
        sender.writer.frame_into(&first_sent, &mut stream);
        let second_at = stream.len();
        let second_sent = frame::contents(&second, Some(64)).unwrap();
        sender.writer.frame_into(&second_sent, &mut stream);

        // The first frame and 3 bytes of the second: the reader keeps those 3
        // at the end of its buffer.
        let mut input = &stream[..second_at + 3];
        while !input.is_empty() {
            reader.read_from(&mut input).unwrap();
        }
        let held = reader.buffered_framed().unwrap().expect("the first packet");
        assert!(reader.buffered_framed().unwrap().is_none());
        // The rest is far larger than the buffer: it is moved to the front and grown.
        let mut input = &stream[second_at + 3..];
        while !input.is_empty() {
            reader.read_from(&mut input).unwrap();
        }
        let next = reader
            .buffered_framed()
            .unwrap()
            .expect("the second packet");
        assert_eq!((next.id, &next.payload[..]), (2, &second[1..]));
        assert_eq!(next.compressed, Some(second_sent));
        assert_eq!((held.id, &held.payload[..]), (1, &first[1..]));
        assert_eq!(held.compressed, Some(first_sent));
    }

    /// Frames that lie about their size, or declare one under the threshold,
    /// are refused by the framed reads with the error `recv` and `buffered`
    /// give, and counted as they are.
    #[test]
    fn framed_reads_refuse_what_recv_refuses() {
        let body = body_of(1, 200);
        let sent = frame::contents(&body, Some(64)).unwrap();
        let mut size_bytes = Vec::new();
        crate::buf::write_varint(&mut size_bytes, body.len() as i32);
        let stream = &sent[size_bytes.len()..];
        let declaring = |size: i32| {
            let mut contents = Vec::new();
            crate::buf::write_varint(&mut contents, size);
            contents.extend_from_slice(stream);
            contents
        };
        let small = frame::contents(&body_of(1, 10), Some(1)).unwrap();
        let cases = [
            (declaring(150), "Badly compressed packet"),
            (declaring(250), "Badly compressed packet"),
            (
                declaring(frame::MAX_UNCOMPRESSED as i32 + 1),
                "protocol maximum",
            ),
            (small, "below server threshold of 64"),
        ];
        for (contents, message) in cases {
            let refusals: Vec<(String, i64)> = (0..4)
                .map(|how| {
                    let (a, b) = Pipe::pair();
                    let mut sender = Connection::local(b);
                    sender.set_compression(64);
                    sender.writer.send_contents(&contents).unwrap();
                    let mut c = Connection::local(a);
                    c.set_compression(64);
                    let counters = c.reader.counters.clone();
                    let error = match how {
                        0 => c.reader.recv().unwrap_err(),
                        1 => c.reader.recv_framed().unwrap_err(),
                        _ => {
                            let mut bytes = Vec::new();
                            sender.writer.frame_into(&contents, &mut bytes);
                            let (_, mut reader, _) = c.detach();
                            reader.read_from(&mut &bytes[..]).unwrap();
                            if how == 2 {
                                reader.buffered().unwrap_err()
                            } else {
                                reader.buffered_framed().unwrap_err()
                            }
                        }
                    };
                    let received = counters.received.load(Ordering::Relaxed);
                    (error.to_string(), received)
                })
                .collect();
            assert!(refusals[0].0.contains(message), "{}", refusals[0].0);
            assert!(refusals.iter().all(|r| *r == refusals[0]), "{refusals:?}");
        }
    }

    /// A login disconnect (id 0) whose text is long enough to be compressed.
    fn login_disconnect() -> Vec<u8> {
        let mut w = crate::buf::Writer::new();
        w.varint(crate::ids::login::clientbound::LOGIN_DISCONNECT);
        w.string(&format!(
            "{{\"text\":\"{}\"}}",
            "closed for the night ".repeat(8)
        ));
        w.data
    }
    /// A login hello (id 0) from a player.
    fn login_hello() -> Vec<u8> {
        let mut w = crate::buf::Writer::new();
        w.varint(crate::ids::login::serverbound::HELLO);
        w.string("Kim_the_long_one");
        w.data.extend_from_slice(&[7; 16]);
        w.data
    }
    /// A connection to a server of an older release, past its handshake (in login), and the
    /// server's end of the pipe.
    fn adapted() -> (Connection, Connection) {
        let (a, b) = Pipe::pair();
        let mut c = Connection::local(a);
        c.adapt(crate::adapter::ServerVersion::V26_2);
        let mut intention = crate::buf::Writer::new();
        intention.varint(crate::ids::handshake::serverbound::INTENTION);
        intention.varint(crate::PROTOCOL_VERSION);
        intention.string("localhost");
        intention.u16(25565);
        intention.varint(2);
        c.send(&intention.data).unwrap();
        c.set_compression(16);
        let mut server = Connection::local(b);
        assert_eq!(server.recv().unwrap().0, 0, "the handshake");
        server.set_compression(16);
        (c, server)
    }

    /// `recv_framed` on a connection that renumbers what arrives gives no frame, a version
    /// adapter's packet or an older client's: the packet it returns is not the frame's.
    #[test]
    fn recv_framed_gives_no_frame_for_a_renumbered_packet() {
        let (mut c, mut server) = adapted();
        server.send(&login_disconnect()).unwrap();
        let got = c.reader.recv_framed().unwrap();
        assert_eq!(got.id, crate::ids::login::clientbound::LOGIN_DISCONNECT);
        assert_eq!(got.compressed, None, "a version adapter's packet");

        let (a, b) = Pipe::pair();
        let mut client = Connection::local(b);
        client.set_compression(16);
        let mut c = Connection::local(a);
        c.serve_older(crate::adapter::ServerVersion::V26_2);
        c.set_compression(16);
        client.send(&login_hello()).unwrap();
        let got = c.reader.recv_framed().unwrap();
        assert_eq!(got.id, crate::ids::login::serverbound::HELLO);
        assert_eq!(got.compressed, None, "an older client's packet");
    }

    /// `buffered_framed` follows `buffered`: an older client's packet (renumbered) has no frame;
    /// with a version adapter, which `buffered` does not apply, the packet comes as it arrived and
    /// so does its frame.
    #[test]
    fn buffered_framed_gives_a_frame_only_for_a_packet_as_it_arrived() {
        let (a, b) = Pipe::pair();
        let mut client = Connection::local(b);
        client.set_compression(16);
        let mut c = Connection::local(a);
        c.serve_older(crate::adapter::ServerVersion::V26_2);
        c.set_compression(16);
        let mut bytes = Vec::new();
        let hello = login_hello();
        client
            .writer
            .frame_into(&frame::contents(&hello, Some(16)).unwrap(), &mut bytes);
        let (_, mut reader, _) = c.detach();
        reader.read_from(&mut &bytes[..]).unwrap();
        let got = reader.buffered_framed().unwrap().expect("the hello");
        assert_eq!(got.id, crate::ids::login::serverbound::HELLO);
        assert_eq!(got.compressed, None, "an older client's packet");

        let (c, mut server) = adapted();
        let disconnect = login_disconnect();
        let sent = frame::contents(&disconnect, Some(16)).unwrap();
        let mut bytes = Vec::new();
        server.writer.frame_into(&sent, &mut bytes);
        let (_, mut reader, _) = c.detach();
        reader.read_from(&mut &bytes[..]).unwrap();
        let got = reader.buffered_framed().unwrap().expect("the disconnect");
        assert_eq!((got.id, &got.payload[..]), (0, &disconnect[1..]));
        assert_eq!(
            got.compressed,
            Some(sent),
            "a version adapter's packet, unchanged"
        );
    }

    #[test]
    fn tcp_stream_enables_nodelay_and_keepalive() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let _server = listener.accept().unwrap();
        assert!(Connection::from_stream(stream).is_ok());
    }
}
