//! A public address for a world that runs at home: a frontend anyone can
//! run on a machine with a public IP, under a wildcard domain they own
//! (`*.mc.example.net`). A host behind NAT dials out to its control port
//! and registers a name; players, on any Minecraft client, join
//! `name.mc.example.net`, and the gateway reads the address in their
//! handshake, asks that host for a fresh connection and copies bytes both
//! ways, as `horizon-relay` does for Play with Friends.
//!
//! Unlike the relay, what it copies is the plain Minecraft protocol, so the
//! operator's machine can read what the protocol doesn't encrypt: always
//! the handshake and the player's name; in offline mode the whole game
//! (online mode encrypts everything after login end to end, between the
//! client and the host). State is in memory; nothing about players is
//! logged. See docs/public-address.md.
pub mod agent;
pub mod handshake;

use handshake::Intent;
use horizon_friends::wire;
use horizon_relay::limits::{Bucket, Limiter, Rates};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    io::{self, Read, Write},
    net::{Shutdown, SocketAddr, TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc::{Receiver, RecvTimeoutError, Sender, SyncSender, channel, sync_channel},
    },
    time::{Duration, Instant},
};

/// Control protocol version.
pub const VERSION: u32 = 1;

/// Names nobody can register (they look like the operator's own).
const RESERVED: &[&str] = &[
    "www", "api", "admin", "status", "mail", "control", "gateway",
];

/// A name is one DNS label: 3 to 32 of `a-z 0-9 -`, no dash at either end.
pub fn valid_name(name: &str) -> bool {
    (3..=32).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !RESERVED.contains(&name)
}

/// Lines a host sends on the control port.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ToGateway {
    /// Register `name`; the connection stays open as the control connection.
    /// `claim` retakes a name this host held before (from `registered`);
    /// `key` is the operator's admission key when the gateway asks for one.
    Host {
        v: u32,
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        claim: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        key: Option<String>,
    },
    /// The host's new connection for a player the gateway announced.
    Attach {
        v: u32,
        session: u64,
        token: String,
    },
    Pong,
}

/// Lines the gateway sends.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum FromGateway {
    Registered {
        name: String,
        /// What players type.
        address: String,
        token: String,
        /// Keeps the name for this host for a while after it disconnects.
        claim: String,
    },
    /// A player is joining: attach a new connection for the session.
    Join {
        session: u64,
    },
    /// The connection now carries the player's stream, from its handshake.
    Bridged {
        session: u64,
    },
    Ping,
    Error {
        code: ErrorCode,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    BadName,
    NameTaken,
    Unauthorized,
    Full,
    RateLimited,
    BadRequest,
}

#[derive(Clone, Debug)]
pub struct Config {
    /// The wildcard domain players' addresses end in.
    pub domain: String,
    /// The Minecraft port players reach (left out of addresses when 25565).
    pub public_port: u16,
    /// Admission keys; when set, only hosts with one may register.
    pub host_keys: Option<HashSet<String>>,
    pub max_hosts: usize,
    pub max_bridges: usize,
    pub max_pending_per_host: usize,
    pub max_connections: usize,
    /// Bytes a second each way per player.
    pub session_rate: f64,
    /// Bytes a second for everything bridged.
    pub total_rate: f64,
    pub rates: Rates,
    pub per_address: bool,
    pub idle: Duration,
    pub ping_every: Duration,
    pub host_timeout: Duration,
    pub first_line: Duration,
    pub attach_wait: Duration,
    /// How long a disconnected host's name stays its own.
    pub name_kept: Duration,
}
impl Config {
    pub fn new(domain: &str) -> Self {
        Self {
            domain: domain.trim().trim_end_matches('.').to_ascii_lowercase(),
            public_port: 25565,
            host_keys: None,
            max_hosts: 500,
            max_bridges: 200,
            max_pending_per_host: 8,
            max_connections: 4096,
            session_rate: 1024.0 * 1024.0,
            total_rate: 100.0 * 1_000_000.0 / 8.0,
            // A server list refresh is one connection, a join two: more
            // than the relay's joins, as every list refresh counts.
            rates: Rates {
                join_per_min: 60.0,
                join_burst: 20.0,
                ..Rates::default()
            },
            per_address: true,
            idle: Duration::from_secs(120),
            ping_every: Duration::from_secs(20),
            host_timeout: Duration::from_secs(60),
            first_line: Duration::from_secs(10),
            attach_wait: Duration::from_secs(10),
            name_kept: Duration::from_secs(24 * 3600),
        }
    }
    /// What players type for `name`.
    pub fn address(&self, name: &str) -> String {
        match self.public_port {
            25565 => format!("{name}.{}", self.domain),
            p => format!("{name}.{}:{p}", self.domain),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub hosts: usize,
    pub bridges: usize,
    pub joins: u64,
    pub unknown: u64,
    pub rate_limited: u64,
    pub bridged_bytes: u64,
}
impl Counts {
    pub fn json(&self) -> String {
        format!(
            "{{\"ok\":true,\"hosts\":{},\"bridges\":{},\"joins\":{},\"unknown\":{},\"rate_limited\":{},\"bridged_bytes\":{}}}",
            self.hosts,
            self.bridges,
            self.joins,
            self.unknown,
            self.rate_limited,
            self.bridged_bytes
        )
    }
}

enum Out {
    Line(FromGateway),
    Closed,
}
struct HostEntry {
    token: String,
    to_host: Sender<Out>,
    pending: usize,
}
#[derive(Default)]
struct Inner {
    hosts: HashMap<String, HostEntry>,
    /// Every name given out and its claim, with when its host left (None
    /// while it is connected).
    claims: HashMap<String, (String, Option<Instant>)>,
    pending: HashMap<u64, (String, SyncSender<TcpStream>)>,
}

struct State {
    config: Config,
    inner: Mutex<Inner>,
    limiter: Mutex<Limiter>,
    total: Mutex<Bucket>,
    bridges: AtomicUsize,
    connections: AtomicUsize,
    joins: AtomicU64,
    unknown: AtomicU64,
    rate_limited: AtomicU64,
    bridged_bytes: AtomicU64,
    stop: AtomicBool,
}

#[derive(Clone)]
pub struct Gateway {
    state: Arc<State>,
}

fn random_token() -> String {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).expect("OS random generator");
    b.iter().map(|x| format!("{x:02x}")).collect()
}
fn random_u64() -> u64 {
    let mut b = [0u8; 8];
    getrandom::fill(&mut b).expect("OS random generator");
    u64::from_le_bytes(b)
}

/// IPv4 clients of a dual-stack listener show up as `::ffff:a.b.c.d`.
fn unmap(peer: SocketAddr) -> SocketAddr {
    match peer {
        SocketAddr::V6(v6) => match v6.ip().to_ipv4_mapped() {
            Some(v4) => SocketAddr::new(v4.into(), v6.port()),
            None => peer,
        },
        v4 => v4,
    }
}

impl Gateway {
    pub fn new(config: Config) -> Self {
        let now = Instant::now();
        Self {
            state: Arc::new(State {
                limiter: Mutex::new(Limiter::new(config.rates.clone(), config.per_address)),
                total: Mutex::new(Bucket::new(config.total_rate, config.total_rate / 4.0, now)),
                config,
                inner: Mutex::new(Inner::default()),
                bridges: AtomicUsize::new(0),
                connections: AtomicUsize::new(0),
                joins: AtomicU64::new(0),
                unknown: AtomicU64::new(0),
                rate_limited: AtomicU64::new(0),
                bridged_bytes: AtomicU64::new(0),
                stop: AtomicBool::new(false),
            }),
        }
    }
    pub fn counts(&self) -> Counts {
        let s = &self.state;
        Counts {
            hosts: s.inner.lock().unwrap().hosts.len(),
            bridges: s.bridges.load(Ordering::Relaxed),
            joins: s.joins.load(Ordering::Relaxed),
            unknown: s.unknown.load(Ordering::Relaxed),
            rate_limited: s.rate_limited.load(Ordering::Relaxed),
            bridged_bytes: s.bridged_bytes.load(Ordering::Relaxed),
        }
    }
    pub fn stop(&self) {
        self.state.stop.store(true, Ordering::Relaxed);
    }
    /// Serve players on `minecraft` and hosts on `control` until
    /// [`Gateway::stop`]; returns when both loops end.
    pub fn serve(&self, minecraft: TcpListener, control: TcpListener) {
        let s = self.state.clone();
        let players = std::thread::Builder::new()
            .name("gateway-players".into())
            .spawn(move || accept(&s, minecraft, player))
            .expect("spawn the players' listener");
        accept(&self.state, control, host_connection);
        let _ = players.join();
    }
}

fn accept(
    state: &Arc<State>,
    listener: TcpListener,
    handler: fn(&Arc<State>, TcpStream, SocketAddr) -> io::Result<()>,
) {
    let _ = listener.set_nonblocking(true);
    while !state.stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, peer)) => {
                // Beyond the cap the connection is closed unanswered.
                if state.connections.fetch_add(1, Ordering::Relaxed) >= state.config.max_connections
                {
                    state.connections.fetch_sub(1, Ordering::Relaxed);
                    continue;
                }
                let st = state.clone();
                let spawned = std::thread::Builder::new()
                    .name("gateway-conn".into())
                    .spawn(move || {
                        let _ = stream.set_nonblocking(false);
                        let _ = handler(&st, stream, unmap(peer));
                        st.connections.fetch_sub(1, Ordering::Relaxed);
                    });
                if spawned.is_err() {
                    state.connections.fetch_sub(1, Ordering::Relaxed);
                }
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(_) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
}

fn refuse(stream: &mut TcpStream, code: ErrorCode) -> io::Result<()> {
    wire::send(stream, &FromGateway::Error { code })
}

/// The control port: a host registering, a host attaching a player's
/// connection, or `GET /health`.
fn host_connection(state: &Arc<State>, mut stream: TcpStream, peer: SocketAddr) -> io::Result<()> {
    let first_line = horizon_friends::deadline(&stream, state.config.first_line)?;
    stream.set_read_timeout(Some(state.config.first_line))?;
    let _ = stream.set_nodelay(true);
    let line = wire::read_line(&mut stream)?.ok_or(io::ErrorKind::UnexpectedEof)?;
    if line.starts_with(b"GET /health") {
        // Drain the request headers (bounded) first: closing with unread
        // bytes would reset the connection before the answer is read.
        for _ in 0..64 {
            match wire::read_line(&mut stream)? {
                Some(h) if !h.is_empty() && h != b"\r" => {}
                _ => break,
            }
        }
        let body = Gateway {
            state: state.clone(),
        }
        .counts()
        .json();
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        return stream.write_all(response.as_bytes());
    }
    drop(first_line);
    match serde_json::from_slice::<ToGateway>(&line) {
        Ok(ToGateway::Host {
            v,
            name,
            claim,
            key,
        }) if v == VERSION => register(state, stream, peer, &name, claim, key),
        Ok(ToGateway::Attach { v, session, token }) if v == VERSION => {
            let target = {
                let inner = state.inner.lock().unwrap();
                inner
                    .pending
                    .get(&session)
                    .filter(|(name, _)| inner.hosts.get(name).is_some_and(|h| h.token == token))
                    .map(|(_, tx)| tx.clone())
            };
            match target {
                Some(tx) => tx.try_send(stream).or_else(|e| {
                    let (std::sync::mpsc::TrySendError::Full(mut s)
                    | std::sync::mpsc::TrySendError::Disconnected(mut s)) = e;
                    refuse(&mut s, ErrorCode::BadRequest)
                }),
                None => refuse(&mut stream, ErrorCode::BadRequest),
            }
        }
        _ => refuse(&mut stream, ErrorCode::BadRequest),
    }
}

fn register(
    state: &Arc<State>,
    mut stream: TcpStream,
    peer: SocketAddr,
    name: &str,
    claim: Option<String>,
    key: Option<String>,
) -> io::Result<()> {
    let now = Instant::now();
    if !state.limiter.lock().unwrap().allow_register(peer.ip(), now) {
        state.rate_limited.fetch_add(1, Ordering::Relaxed);
        return refuse(&mut stream, ErrorCode::RateLimited);
    }
    let admitted = match &state.config.host_keys {
        Some(keys) => key.is_some_and(|k| keys.contains(&k)),
        None => true,
    };
    if !admitted {
        return refuse(&mut stream, ErrorCode::Unauthorized);
    }
    let name = name.to_ascii_lowercase();
    if !valid_name(&name) {
        return refuse(&mut stream, ErrorCode::BadName);
    }
    let (tx, rx) = channel();
    let token = random_token();
    let granted = {
        let mut inner = state.inner.lock().unwrap();
        let kept = state.config.name_kept;
        inner
            .claims
            .retain(|_, (_, left)| left.is_none_or(|t| now.saturating_duration_since(t) < kept));
        let mine = match inner.claims.get(&name) {
            None => Ok(random_token()),
            // Only the name's claim takes it, also from a connection the
            // gateway hasn't noticed is dead yet (the old one is closed).
            Some((c, _)) if claim.as_deref() == Some(c.as_str()) => Ok(c.clone()),
            Some(_) => Err(ErrorCode::NameTaken),
        };
        if mine.is_ok()
            && let Some(old) = inner.hosts.remove(&name)
        {
            let _ = old.to_host.send(Out::Closed);
        }
        match mine {
            Ok(_) if inner.hosts.len() >= state.config.max_hosts => Err(ErrorCode::Full),
            Ok(c) => {
                inner.claims.insert(name.clone(), (c.clone(), None));
                inner.hosts.insert(
                    name.clone(),
                    HostEntry {
                        token: token.clone(),
                        to_host: tx.clone(),
                        pending: 0,
                    },
                );
                Ok(c)
            }
            Err(e) => Err(e),
        }
    };
    let claim = match granted {
        Ok(c) => c,
        Err(code) => return refuse(&mut stream, code),
    };
    let result = serve_host(state, &mut stream, &name, &token, &claim, tx, rx);
    {
        let mut inner = state.inner.lock().unwrap();
        if inner.hosts.get(&name).is_some_and(|h| h.token == token) {
            inner.hosts.remove(&name);
            inner.claims.insert(name, (claim, Some(Instant::now())));
        }
    }
    let _ = stream.shutdown(Shutdown::Both);
    result
}

/// Joins and pings out, pongs in; a host silent for `host_timeout` is gone.
fn serve_host(
    state: &Arc<State>,
    stream: &mut TcpStream,
    name: &str,
    token: &str,
    claim: &str,
    tx: Sender<Out>,
    rx: Receiver<Out>,
) -> io::Result<()> {
    wire::send(
        stream,
        &FromGateway::Registered {
            name: name.to_owned(),
            address: state.config.address(name),
            token: token.to_owned(),
            claim: claim.to_owned(),
        },
    )?;
    let mut reader = stream.try_clone()?;
    reader.set_read_timeout(Some(state.config.host_timeout))?;
    std::thread::Builder::new()
        .name("gateway-host-read".into())
        .spawn(move || {
            while let Ok(Some(_)) = wire::read_line(&mut reader) {}
            let _ = tx.send(Out::Closed);
        })?;
    while !state.stop.load(Ordering::Relaxed) {
        match rx.recv_timeout(state.config.ping_every) {
            Ok(Out::Line(m)) => wire::send(stream, &m)?,
            Ok(Out::Closed) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => wire::send(stream, &FromGateway::Ping)?,
        }
    }
    Ok(())
}

/// What the gateway says itself when no host takes the player.
fn answer(stream: &mut TcpStream, intent: Intent, protocol: i32, text: &str) -> io::Result<()> {
    match intent {
        Intent::Status => handshake::answer_status(stream, protocol, text),
        Intent::Login | Intent::Transfer => handshake::refuse_login(stream, text),
    }
}

/// The Minecraft port: read the handshake, find the host its address
/// names, bridge.
fn player(state: &Arc<State>, mut stream: TcpStream, peer: SocketAddr) -> io::Result<()> {
    let opening = horizon_friends::deadline(&stream, state.config.first_line)?;
    stream.set_read_timeout(Some(state.config.first_line))?;
    let _ = stream.set_nodelay(true);
    let hello = handshake::read(&mut stream)?;
    let now = Instant::now();
    state.joins.fetch_add(1, Ordering::Relaxed);
    if !state.limiter.lock().unwrap().allow_join(peer.ip(), now) {
        state.rate_limited.fetch_add(1, Ordering::Relaxed);
        return answer(
            &mut stream,
            hello.intent,
            hello.protocol,
            "Too many connections, try again in a minute",
        );
    }
    let host = handshake::normalize(&hello.address);
    let Some(name) = handshake::route(&host, &state.config.domain).map(str::to_owned) else {
        state.unknown.fetch_add(1, Ordering::Relaxed);
        return answer(
            &mut stream,
            hello.intent,
            hello.protocol,
            "No world has this address",
        );
    };
    let session = random_u64();
    let (atx, arx) = sync_channel(1);
    let found = {
        let mut inner = state.inner.lock().unwrap();
        let busy = state.bridges.load(Ordering::Relaxed) >= state.config.max_bridges;
        let held = inner.claims.contains_key(&name);
        let found = match inner.hosts.get_mut(&name) {
            Some(h) if !busy && h.pending < state.config.max_pending_per_host => {
                h.pending += 1;
                Ok(h.to_host.clone())
            }
            Some(_) => Err("This gateway is full, try again in a minute"),
            None if held => Err("This world is offline"),
            None => Err("No world has this address"),
        };
        if found.is_ok() {
            inner.pending.insert(session, (name.clone(), atx));
        }
        found
    };
    let to_host = match found {
        Ok(tx) => tx,
        Err(text) => {
            if text.starts_with("No world") {
                state.unknown.fetch_add(1, Ordering::Relaxed);
                state.limiter.lock().unwrap().unknown(peer.ip(), now);
            }
            return answer(&mut stream, hello.intent, hello.protocol, text);
        }
    };
    // From here the wait is bounded by `attach_wait`, not the opening's.
    drop(opening);
    let _ = to_host.send(Out::Line(FromGateway::Join { session }));
    let attached = arx.recv_timeout(state.config.attach_wait);
    {
        let mut inner = state.inner.lock().unwrap();
        inner.pending.remove(&session);
        if let Some(h) = inner.hosts.get_mut(&name) {
            h.pending = h.pending.saturating_sub(1);
        }
    }
    let Ok(mut host) = attached else {
        return answer(
            &mut stream,
            hello.intent,
            hello.protocol,
            "The world didn't answer",
        );
    };
    state.bridges.fetch_add(1, Ordering::Relaxed);
    let result = (|| -> io::Result<()> {
        wire::send(&mut host, &FromGateway::Bridged { session })?;
        // The host's server reads the player's handshake as if it had come
        // straight to it.
        host.write_all(&hello.raw)?;
        for s in [&stream, &host] {
            s.set_read_timeout(Some(state.config.idle))?;
            let _ = s.set_nodelay(true);
        }
        let (p, h, st) = (stream.try_clone()?, host.try_clone()?, state.clone());
        let up = std::thread::Builder::new()
            .name("gateway-copy".into())
            .spawn(move || copy(&st, p, h))?;
        copy(state, host, stream);
        let _ = up.join();
        Ok(())
    })();
    state.bridges.fetch_sub(1, Ordering::Relaxed);
    result
}

fn copy(state: &State, mut from: TcpStream, mut to: TcpStream) {
    let rate = state.config.session_rate;
    let mut bucket = Bucket::new(rate, (rate / 2.0).max(16384.0), Instant::now());
    let mut buf = vec![0u8; 16 * 1024];
    loop {
        let n = match from.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        let now = Instant::now();
        let wait = bucket
            .take(n as f64, now)
            .max(state.total.lock().unwrap().take(n as f64, now));
        if !wait.is_zero() {
            std::thread::sleep(wait);
        }
        if to.write_all(&buf[..n]).is_err() {
            break;
        }
        state.bridged_bytes.fetch_add(n as u64, Ordering::Relaxed);
    }
    let _ = from.shutdown(Shutdown::Both);
    let _ = to.shutdown(Shutdown::Both);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        for ok in ["pullmon", "abc", "my-world-2", &"a".repeat(32)] {
            assert!(valid_name(ok), "{ok}");
        }
        for bad in [
            "ab",
            "-abc",
            "abc-",
            "PullMon",
            "a.b",
            "a_b",
            "www",
            &"a".repeat(33),
            "wörld",
        ] {
            assert!(!valid_name(bad), "{bad}");
        }
    }

    #[test]
    fn addresses() {
        let mut c = Config::new("MC.Example.net.");
        assert_eq!(c.address("pullmon"), "pullmon.mc.example.net");
        c.public_port = 25570;
        assert_eq!(c.address("pullmon"), "pullmon.mc.example.net:25570");
    }
}
