//! The host's side: keep a control connection to a gateway, and for every
//! player it announces open a new connection to it and one to the local
//! server (the integrated server's LAN port, or a dedicated server), then
//! copy bytes between them. Only outbound connections: works behind any
//! NAT the host can browse the web from.
use crate::{ErrorCode, FromGateway, ToGateway, VERSION};
use horizon_friends::wire;
use std::{
    io::{self, BufReader},
    net::{Shutdown, TcpStream, ToSocketAddrs},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::Duration,
};

#[derive(Clone, Debug)]
pub struct ExposeConfig {
    /// The gateway's control port, `host:port`.
    pub gateway: String,
    pub name: String,
    /// The server players reach, usually `127.0.0.1:<port>`.
    pub to: String,
    pub claim: Option<String>,
    pub key: Option<String>,
}

#[derive(Debug)]
pub enum ExposeError {
    Io(io::Error),
    Refused(ErrorCode),
}
impl std::fmt::Display for ExposeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Refused(ErrorCode::NameTaken) => f.write_str("someone else has this name"),
            Self::Refused(ErrorCode::BadName) => {
                f.write_str("a name is 3 to 32 of a-z, 0-9 and -, not starting or ending with -")
            }
            Self::Refused(ErrorCode::Unauthorized) => {
                f.write_str("this gateway needs a key from its operator")
            }
            Self::Refused(ErrorCode::Full) => f.write_str("the gateway is full"),
            Self::Refused(ErrorCode::RateLimited) => {
                f.write_str("too many attempts, wait a minute")
            }
            Self::Refused(ErrorCode::BadRequest) => {
                f.write_str("the gateway didn't understand the request")
            }
        }
    }
}
impl From<io::Error> for ExposeError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// A registered name, served until [`Exposed::stop`] or until the gateway
/// goes away ([`Exposed::wait`] returns then).
pub struct Exposed {
    pub address: String,
    pub claim: String,
    control: TcpStream,
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}
impl Exposed {
    pub fn stop(self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.control.shutdown(Shutdown::Both);
        let _ = self.thread.join();
    }
    /// Block until the control connection ends.
    pub fn wait(self) {
        let _ = self.thread.join();
    }
}

fn connect(address: &str) -> io::Result<TcpStream> {
    let mut last = io::Error::new(io::ErrorKind::NotFound, "no address");
    for a in address.to_socket_addrs()? {
        match TcpStream::connect_timeout(&a, Duration::from_secs(10)) {
            Ok(s) => {
                let _ = s.set_nodelay(true);
                return Ok(s);
            }
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// Register and start serving.
pub fn expose(config: ExposeConfig) -> Result<Exposed, ExposeError> {
    let mut control = connect(&config.gateway)?;
    control.set_read_timeout(Some(Duration::from_secs(15)))?;
    wire::send(
        &mut control,
        &ToGateway::Host {
            v: VERSION,
            name: config.name.clone(),
            claim: config.claim.clone(),
            key: config.key.clone(),
        },
    )?;
    let mut reader = BufReader::new(control.try_clone()?);
    let (address, token, claim) = match wire::recv_buffered(&mut reader)? {
        FromGateway::Registered {
            address,
            token,
            claim,
            ..
        } => (address, token, claim),
        FromGateway::Error { code } => return Err(ExposeError::Refused(code)),
        _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "unexpected answer").into()),
    };
    // The gateway pings every 20 s; a minute of silence means it is gone.
    control.set_read_timeout(Some(Duration::from_secs(60)))?;
    let stop = Arc::new(AtomicBool::new(false));
    let (st, mut writer) = (stop.clone(), control.try_clone()?);
    let thread = std::thread::Builder::new()
        .name("gateway-agent".into())
        .spawn(move || {
            while !st.load(Ordering::Relaxed) {
                match wire::recv_buffered::<_, FromGateway>(&mut reader) {
                    Ok(FromGateway::Ping) => {
                        if wire::send(&mut writer, &ToGateway::Pong).is_err() {
                            break;
                        }
                    }
                    Ok(FromGateway::Join { session }) => {
                        let (gateway, to, token) =
                            (config.gateway.clone(), config.to.clone(), token.clone());
                        let _ = std::thread::Builder::new()
                            .name("gateway-player".into())
                            .spawn(move || {
                                let _ = attach(&gateway, &to, session, &token);
                            });
                    }
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
        })?;
    Ok(Exposed {
        address,
        claim,
        control,
        stop,
        thread,
    })
}

fn attach(gateway: &str, to: &str, session: u64, token: &str) -> io::Result<()> {
    let mut up = connect(gateway)?;
    up.set_read_timeout(Some(Duration::from_secs(15)))?;
    wire::send(
        &mut up,
        &ToGateway::Attach {
            v: VERSION,
            session,
            token: token.to_owned(),
        },
    )?;
    // Read the line a byte at a time: the player's bytes follow it.
    match wire::recv::<_, FromGateway>(&mut up)? {
        FromGateway::Bridged { .. } => {}
        _ => return Ok(()),
    }
    up.set_read_timeout(None)?;
    let local = connect(to)?;
    let (mut a, mut b) = (up.try_clone()?, local.try_clone()?);
    let down = std::thread::spawn(move || {
        let _ = io::copy(&mut b, &mut a);
        let _ = a.shutdown(Shutdown::Both);
        let _ = b.shutdown(Shutdown::Both);
    });
    let (mut up_r, mut local_w) = (up, local);
    let _ = io::copy(&mut up_r, &mut local_w);
    let _ = up_r.shutdown(Shutdown::Both);
    let _ = local_w.shutdown(Shutdown::Both);
    let _ = down.join();
    Ok(())
}
