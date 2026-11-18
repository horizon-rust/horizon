//! A packet body on its way to a client: its own bytes, or bytes several clients share.

use std::sync::Arc;

/// A packet body (VarInt id and payload) on its way to one client. Entity tracking sends the
/// same bodies to every client tracking an entity: those hold one shared copy, where a
/// body sent to one client keeps its own bytes without copying them again. Either goes on
/// the wire the same way.
#[derive(Clone)]
pub enum Body {
    Owned(Vec<u8>),
    Shared(Arc<[u8]>),
    /// Several packets of a tick that every client tracking a run of entities is sent: a range
    /// of one buffer built once, the packets in it framed one by one when it is written.
    Run(Run),
}

/// The packets of one tick's tracked entities' changes, back to back, with where each ends.
#[derive(Default)]
pub struct RunBuf {
    bytes: Vec<u8>,
    ends: Vec<u32>,
}

impl RunBuf {
    /// Adds a packet.
    pub fn push(&mut self, packet: &[u8]) {
        self.bytes.extend_from_slice(packet);
        self.ends.push(self.bytes.len() as u32);
    }
    /// Packet `i`.
    pub fn packet(&self, i: u32) -> &[u8] {
        let start = match i {
            0 => 0,
            n => self.ends[n as usize - 1] as usize,
        };
        &self.bytes[start..self.ends[i as usize] as usize]
    }
    /// The number of packets so far: where the next one will be.
    pub fn mark(&self) -> u32 {
        self.ends.len() as u32
    }
}

/// Packets `from..to` of a [`RunBuf`].
#[derive(Clone)]
pub struct Run {
    buf: Arc<RunBuf>,
    from: u32,
    to: u32,
}

impl Run {
    /// The packets `from..to` of `buf`.
    pub fn new(buf: &Arc<RunBuf>, from: u32, to: u32) -> Self {
        Self {
            buf: Arc::clone(buf),
            from,
            to,
        }
    }
    /// Adds the packets `from..to` of `buf` to this run when they come right after it in the
    /// same buffer: whether they did.
    pub fn extend(&mut self, buf: &Arc<RunBuf>, from: u32, to: u32) -> bool {
        if Arc::ptr_eq(&self.buf, buf) && self.to == from {
            self.to = to;
            return true;
        }
        false
    }
    fn start(&self) -> usize {
        match self.from {
            0 => 0,
            n => self.buf.ends[n as usize - 1] as usize,
        }
    }
    fn end(&self) -> usize {
        match self.to {
            0 => 0,
            n => self.buf.ends[n as usize - 1] as usize,
        }
    }
    /// This run as a body.
    pub fn into_body(self) -> Body {
        Body::Run(self)
    }
    /// Packet `i` of the buffer.
    pub fn packet(&self, i: u32) -> &[u8] {
        self.buf.packet(i)
    }
    /// The buffer and the packets `from..to` of it.
    pub fn parts(&self) -> (&Arc<RunBuf>, u32, u32) {
        (&self.buf, self.from, self.to)
    }
    /// The packets in order.
    pub fn packets(&self) -> impl Iterator<Item = &[u8]> {
        (self.from..self.to).map(|i| {
            let start = match i {
                0 => 0,
                n => self.buf.ends[n as usize - 1] as usize,
            };
            &self.buf.bytes[start..self.buf.ends[i as usize] as usize]
        })
    }
}

impl Body {
    /// Bytes several clients will be sent.
    pub fn shared(bytes: Vec<u8>) -> Self {
        Self::Shared(bytes.into())
    }
    /// The bytes this body keeps alive: an owned vector's capacity, a shared copy's length.
    pub fn retained(&self) -> usize {
        match self {
            Self::Owned(bytes) => bytes.capacity(),
            Self::Shared(bytes) => bytes.len(),
            Self::Run(run) => run.end() - run.start(),
        }
    }
    /// The packets this body is: itself, or those of a run.
    pub fn packets(&self) -> Vec<&[u8]> {
        match self {
            Self::Run(run) => run.packets().collect(),
            one => vec![&**one],
        }
    }
    /// `bodies` as the packets they hold, each its own bytes.
    pub fn flatten(bodies: &[Body]) -> Vec<Vec<u8>> {
        bodies
            .iter()
            .flat_map(|b| b.packets().into_iter().map(<[u8]>::to_vec))
            .collect()
    }
    /// The bytes as a vector of their own (copied when shared).
    pub fn into_vec(self) -> Vec<u8> {
        match self {
            Self::Owned(bytes) => bytes,
            Self::Shared(bytes) => bytes.to_vec(),
            Self::Run(run) => run.buf.bytes[run.start()..run.end()].to_vec(),
        }
    }
}

impl std::ops::Deref for Body {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        match self {
            Self::Owned(bytes) => bytes,
            Self::Shared(bytes) => bytes,
            Self::Run(run) => &run.buf.bytes[run.start()..run.end()],
        }
    }
}

impl From<Vec<u8>> for Body {
    fn from(bytes: Vec<u8>) -> Self {
        Self::Owned(bytes)
    }
}

/// Bodies are equal when their bytes are, shared or not.
impl PartialEq for Body {
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}
impl Eq for Body {}
impl PartialEq<Vec<u8>> for Body {
    fn eq(&self, other: &Vec<u8>) -> bool {
        **self == other[..]
    }
}

impl std::fmt::Debug for Body {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&**self, f)
    }
}
