//! A gateway, hosts behind it and players dialling its Minecraft port, all
//! on loopback in one process. The "server" behind each host is a small
//! stand-in that checks the handshake it receives and then echoes.
use horizon_gateway::{
    Config, ErrorCode, Gateway,
    agent::{self, ExposeConfig, ExposeError},
    handshake::{self, Intent},
};
use std::{
    collections::HashSet,
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::mpsc::{Receiver, channel},
    time::Duration,
};

const DOMAIN: &str = "mc.test";

fn gateway(config: Config) -> (Gateway, SocketAddr, String) {
    let minecraft = TcpListener::bind("127.0.0.1:0").unwrap();
    let control = TcpListener::bind("127.0.0.1:0").unwrap();
    let (m, c) = (
        minecraft.local_addr().unwrap(),
        control.local_addr().unwrap(),
    );
    let g = Gateway::new(config);
    let gc = g.clone();
    std::thread::spawn(move || gc.serve(minecraft, control));
    (g, m, c.to_string())
}

/// A server that reports every handshake it reads, then echoes.
fn server() -> (String, Receiver<handshake::Handshake>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let a = l.local_addr().unwrap().to_string();
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        for s in l.incoming() {
            let Ok(mut s) = s else { continue };
            let tx = tx.clone();
            std::thread::spawn(move || {
                let Ok(h) = handshake::read(&mut s) else {
                    return;
                };
                let _ = tx.send(h);
                let mut out = s.try_clone().unwrap();
                let _ = std::io::copy(&mut s, &mut out);
            });
        }
    });
    (a, rx)
}

fn expose(
    control: &str,
    name: &str,
    to: &str,
    claim: Option<String>,
) -> Result<agent::Exposed, ExposeError> {
    agent::expose(ExposeConfig {
        gateway: control.to_owned(),
        name: name.to_owned(),
        to: to.to_owned(),
        claim,
        key: None,
    })
}

fn dial(gateway: SocketAddr) -> TcpStream {
    let s = TcpStream::connect(gateway).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    s
}

/// Ping the gateway for `address`; the status text it answers itself.
fn status_text(gateway: SocketAddr, address: &str) -> String {
    let mut s = dial(gateway);
    s.write_all(&handshake::encode(777, address, 25565, Intent::Status))
        .unwrap();
    s.write_all(&handshake::frame(0, &[])).unwrap();
    let frame = read_frame(&mut s);
    // Packet id 0, then the JSON as a string.
    assert_eq!(frame[0], 0);
    let start = frame.iter().position(|&b| b == b'{').unwrap();
    let json: serde_json::Value = serde_json::from_slice(&frame[start..]).unwrap();
    json["description"]["text"].as_str().unwrap().to_owned()
}

fn read_frame(s: &mut TcpStream) -> Vec<u8> {
    let (mut len, mut shift) = (0usize, 0);
    loop {
        let mut b = [0u8; 1];
        s.read_exact(&mut b).unwrap();
        len |= usize::from(b[0] & 0x7f) << shift;
        shift += 7;
        if b[0] & 0x80 == 0 {
            break;
        }
    }
    let mut frame = vec![0u8; len];
    s.read_exact(&mut frame).unwrap();
    frame
}

/// Log in to `address`; the disconnect reason the gateway answers.
fn login_refusal(gateway: SocketAddr, address: &str) -> String {
    let mut s = dial(gateway);
    s.write_all(&handshake::encode(777, address, 25565, Intent::Login))
        .unwrap();
    let mut reply = Vec::new();
    let _ = s.read_to_end(&mut reply);
    let start = reply.iter().position(|&b| b == b'{').unwrap();
    let json: serde_json::Value = serde_json::from_slice(&reply[start..]).unwrap();
    json["text"].as_str().unwrap().to_owned()
}

#[test]
fn players_reach_the_host_their_address_names() {
    let (g, mc, control) = gateway(Config::new(DOMAIN));
    let (a, a_seen) = server();
    let (b, b_seen) = server();
    let pullmon = expose(&control, "pullmon", &a, None).unwrap();
    let other = expose(&control, "other", &b, None).unwrap();
    assert_eq!(pullmon.address, "pullmon.mc.test");

    // A vanilla-style login handshake, with the case and the trailing dot
    // clients may send, then a megabyte both ways.
    let sent = handshake::encode(777, "PullMon.mc.test.", 25565, Intent::Login);
    let mut s = dial(mc);
    s.write_all(&sent).unwrap();
    let h = a_seen.recv_timeout(Duration::from_secs(10)).unwrap();
    // The host's server reads exactly what the player sent.
    assert_eq!(h.raw, sent);
    assert_eq!(h.address, "PullMon.mc.test.");
    let data: Vec<u8> = (0..1 << 20).map(|i| (i * 7 % 251) as u8).collect();
    let mut w = s.try_clone().unwrap();
    let d = data.clone();
    let writer = std::thread::spawn(move || w.write_all(&d).unwrap());
    let mut back = vec![0u8; data.len()];
    s.read_exact(&mut back).unwrap();
    writer.join().unwrap();
    assert!(back == data);

    // The other name goes to the other host.
    let mut s2 = dial(mc);
    s2.write_all(&handshake::encode(
        777,
        "other.mc.test",
        25565,
        Intent::Status,
    ))
    .unwrap();
    assert_eq!(
        b_seen
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
            .address,
        "other.mc.test"
    );
    assert!(a_seen.try_recv().is_err());
    assert_eq!(g.counts().hosts, 2);
    pullmon.stop();
    other.stop();
}

#[test]
fn the_gateway_answers_for_names_with_no_world() {
    let (_g, mc, control) = gateway(Config::new(DOMAIN));
    assert_eq!(
        status_text(mc, "nobody.mc.test"),
        "No world has this address"
    );
    assert_eq!(status_text(mc, "mc.test"), "No world has this address");
    assert_eq!(
        login_refusal(mc, "nobody.mc.test"),
        "No world has this address"
    );
    let (a, _) = server();
    let host = expose(&control, "pullmon", &a, None).unwrap();
    host.stop();
    // Give the gateway a moment to see the control connection end.
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(status_text(mc, "pullmon.mc.test"), "This world is offline");
    assert_eq!(
        login_refusal(mc, "pullmon.mc.test"),
        "This world is offline"
    );
}

#[test]
fn a_name_stays_with_its_claim() {
    let mut config = Config::new(DOMAIN);
    config.rates.register_burst = 50.0;
    let (_g, mc, control) = gateway(config);
    let (a, a_seen) = server();
    let first = expose(&control, "pullmon", &a, None).unwrap();
    let claim = first.claim.clone();
    // Taken while its host is connected, and after it leaves.
    assert!(matches!(
        expose(&control, "pullmon", &a, None),
        Err(ExposeError::Refused(ErrorCode::NameTaken))
    ));
    first.stop();
    std::thread::sleep(Duration::from_millis(300));
    assert!(matches!(
        expose(&control, "pullmon", &a, Some("wrong".into())),
        Err(ExposeError::Refused(ErrorCode::NameTaken))
    ));
    // The claim takes it back, the same address, and players get through.
    let again = expose(&control, "pullmon", &a, Some(claim.clone())).unwrap();
    assert_eq!(
        (again.address.as_str(), again.claim.as_str()),
        ("pullmon.mc.test", claim.as_str())
    );
    // A reconnect with the claim replaces a connection not yet seen dead.
    let replaced = expose(&control, "pullmon", &a, Some(claim)).unwrap();
    let mut s = dial(mc);
    s.write_all(&handshake::encode(
        777,
        "pullmon.mc.test",
        25565,
        Intent::Login,
    ))
    .unwrap();
    assert!(a_seen.recv_timeout(Duration::from_secs(10)).is_ok());
    replaced.stop();
    again.stop();
    // Bad names never register.
    for bad in ["ab", "Bad_Name", "www"] {
        assert!(matches!(
            expose(&control, bad, &a, None),
            Err(ExposeError::Refused(ErrorCode::BadName))
        ));
    }
}

#[test]
fn a_keyed_gateway_admits_only_its_keys() {
    let mut config = Config::new(DOMAIN);
    config.host_keys = Some(HashSet::from(["k1".to_owned()]));
    let (_g, _mc, control) = gateway(config);
    let (a, _) = server();
    assert!(matches!(
        expose(&control, "pullmon", &a, None),
        Err(ExposeError::Refused(ErrorCode::Unauthorized))
    ));
    let keyed = agent::expose(ExposeConfig {
        gateway: control.clone(),
        name: "pullmon".into(),
        to: a,
        claim: None,
        key: Some("k1".into()),
    })
    .unwrap();
    keyed.stop();
}

#[test]
fn garbage_and_slow_openings_are_dropped() {
    let mut config = Config::new(DOMAIN);
    config.first_line = Duration::from_millis(500);
    let (g, mc, control) = gateway(config);
    // Not Minecraft.
    let mut s = dial(mc);
    s.write_all(b"GET / HTTP/1.1\r\n\r\n").unwrap();
    let mut rest = Vec::new();
    assert_eq!(s.read_to_end(&mut rest).unwrap_or(0), 0);
    // A handshake trickled a byte at a time is cut off by the deadline.
    let mut slow = dial(mc);
    slow.write_all(&[20]).unwrap();
    std::thread::sleep(Duration::from_millis(900));
    let mut b = [0u8; 1];
    assert!(matches!(slow.read(&mut b), Ok(0) | Err(_)));
    // The control port answers health checks.
    let mut h = TcpStream::connect(&control).unwrap();
    h.write_all(b"GET /health HTTP/1.1\r\n\r\n").unwrap();
    let mut body = String::new();
    h.read_to_string(&mut body).unwrap();
    assert!(body.starts_with("HTTP/1.1 200 OK") && body.contains("\"ok\":true"));
    assert_eq!(g.counts().bridges, 0);
}
