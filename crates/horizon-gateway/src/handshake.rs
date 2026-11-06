//! The one Minecraft packet the gateway reads: the client's intention
//! (`ClientIntentionPacket`: protocol version, the address the player typed,
//! port, next state). It names the world to route to. Everything after it
//! is copied unread. Also the two answers the gateway gives on its own when
//! no host has the name: a status response and a login disconnect.
use std::io::{self, Read, Write};

/// Longest handshake frame accepted: vanilla's address is at most 255
/// characters (`ClientIntentionPacket`'s `readUtf(255)`), up to 3 bytes each.
pub const MAX_FRAME: usize = 1 + 5 + 3 + 255 * 3 + 2 + 5;

/// What the client asks for after the handshake.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Intent {
    Status,
    Login,
    Transfer,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Handshake {
    pub protocol: i32,
    /// The address as the client sent it.
    pub address: String,
    pub port: u16,
    pub intent: Intent,
    /// The whole frame as read, replayed to the host first.
    pub raw: Vec<u8>,
}

fn invalid(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.to_owned())
}

/// Read a VarInt a byte at a time; also returns its bytes.
fn read_varint<R: Read>(r: &mut R, raw: &mut Vec<u8>) -> io::Result<i32> {
    let mut value = 0u32;
    for i in 0..5 {
        let mut b = [0u8; 1];
        r.read_exact(&mut b)?;
        raw.push(b[0]);
        value |= u32::from(b[0] & 0x7f) << (7 * i);
        if b[0] & 0x80 == 0 {
            return Ok(value as i32);
        }
    }
    Err(invalid("VarInt too long"))
}

fn varint_at(buf: &[u8], at: &mut usize) -> io::Result<i32> {
    let mut value = 0u32;
    for i in 0..5 {
        let b = *buf.get(*at).ok_or_else(|| invalid("short packet"))?;
        *at += 1;
        value |= u32::from(b & 0x7f) << (7 * i);
        if b & 0x80 == 0 {
            return Ok(value as i32);
        }
    }
    Err(invalid("VarInt too long"))
}

/// Read the handshake frame. A legacy (pre-1.7) server list ping, which
/// starts with 0xFE, is refused like any other malformed opening.
pub fn read<R: Read>(r: &mut R) -> io::Result<Handshake> {
    let mut raw = Vec::new();
    let len = read_varint(r, &mut raw)?;
    if len <= 0 || len as usize > MAX_FRAME {
        return Err(invalid("not a handshake"));
    }
    let start = raw.len();
    raw.resize(start + len as usize, 0);
    r.read_exact(&mut raw[start..])?;
    let body = &raw[start..];
    let mut at = 0;
    if varint_at(body, &mut at)? != 0 {
        return Err(invalid("not a handshake"));
    }
    let protocol = varint_at(body, &mut at)?;
    let n = varint_at(body, &mut at)?;
    if n < 0 || n as usize > 255 * 3 || at + n as usize + 2 > body.len() {
        return Err(invalid("bad address"));
    }
    let address = std::str::from_utf8(&body[at..at + n as usize])
        .map_err(|_| invalid("bad address"))?
        .to_owned();
    at += n as usize;
    let port = u16::from_be_bytes([body[at], body[at + 1]]);
    at += 2;
    let intent = match varint_at(body, &mut at)? {
        1 => Intent::Status,
        2 => Intent::Login,
        3 => Intent::Transfer,
        _ => return Err(invalid("bad intent")),
    };
    Ok(Handshake {
        protocol,
        address,
        port,
        intent,
        raw,
    })
}

/// The host part the routing looks at: up to the first NUL (Forge and
/// others append markers after one), without a trailing dot, lowercase.
pub fn normalize(address: &str) -> String {
    let host = address.split('\0').next().unwrap_or("");
    host.trim_end_matches('.').to_ascii_lowercase()
}

/// The name a normalized address routes to under `domain`: exactly one DNS
/// label in front of it (`pullmon.mc.example.net` → `pullmon`).
pub fn route<'a>(host: &'a str, domain: &str) -> Option<&'a str> {
    let label = host.strip_suffix(domain)?.strip_suffix('.')?;
    crate::valid_name(label).then_some(label)
}

fn write_varint(out: &mut Vec<u8>, value: i32) {
    let mut v = value as u32;
    loop {
        if v & !0x7f == 0 {
            out.push(v as u8);
            return;
        }
        out.push((v as u8 & 0x7f) | 0x80);
        v >>= 7;
    }
}

fn write_string(out: &mut Vec<u8>, s: &str) {
    write_varint(out, s.len() as i32);
    out.extend_from_slice(s.as_bytes());
}

/// One uncompressed frame: length, packet id, body.
pub fn frame(id: i32, body: &[u8]) -> Vec<u8> {
    let mut packet = Vec::new();
    write_varint(&mut packet, id);
    packet.extend_from_slice(body);
    let mut out = Vec::new();
    write_varint(&mut out, packet.len() as i32);
    out.extend_from_slice(&packet);
    out
}

/// The handshake a client sends (tests, and the agent's own probes).
pub fn encode(protocol: i32, address: &str, port: u16, intent: Intent) -> Vec<u8> {
    let mut body = Vec::new();
    write_varint(&mut body, protocol);
    write_string(&mut body, address);
    body.extend_from_slice(&port.to_be_bytes());
    write_varint(
        &mut body,
        match intent {
            Intent::Status => 1,
            Intent::Login => 2,
            Intent::Transfer => 3,
        },
    );
    frame(0, &body)
}

fn read_frame<R: Read>(r: &mut R, max: usize) -> io::Result<Vec<u8>> {
    let mut scratch = Vec::new();
    let len = read_varint(r, &mut scratch)?;
    if len <= 0 || len as usize > max {
        return Err(invalid("bad frame"));
    }
    let mut body = vec![0u8; len as usize];
    r.read_exact(&mut body)?;
    Ok(body)
}

/// Answer a status request ourselves: the server list shows `motd` with no
/// players, then the ping is echoed as vanilla's server does.
pub fn answer_status<S: Read + Write>(s: &mut S, protocol: i32, motd: &str) -> io::Result<()> {
    if read_frame(s, 1)? != [0] {
        return Err(invalid("expected a status request"));
    }
    let json = serde_json::json!({
        "version": {"name": "26.3", "protocol": protocol},
        "players": {"max": 0, "online": 0},
        "description": {"text": motd},
    })
    .to_string();
    let mut body = Vec::new();
    write_string(&mut body, &json);
    s.write_all(&frame(0, &body))?;
    // The ping: id 1 and a long, sent back unchanged.
    let ping = read_frame(s, 9)?;
    if ping.len() == 9 && ping[0] == 1 {
        s.write_all(&frame(1, &ping[1..]))?;
    }
    s.flush()
}

/// End a login with `reason` (login state's disconnect: a JSON text
/// component).
pub fn refuse_login<W: Write>(w: &mut W, reason: &str) -> io::Result<()> {
    let mut body = Vec::new();
    write_string(
        &mut body,
        &serde_json::json!({ "text": reason }).to_string(),
    );
    w.write_all(&frame(0, &body))?;
    w.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_what_a_client_sends() {
        let bytes = encode(777, "PullMon.mc.example.net.", 25565, Intent::Login);
        let mut stream = bytes.clone();
        stream.extend_from_slice(b"login start");
        let mut r = &stream[..];
        let h = read(&mut r).unwrap();
        assert_eq!(h.protocol, 777);
        assert_eq!(h.address, "PullMon.mc.example.net.");
        assert_eq!(h.port, 25565);
        assert_eq!(h.intent, Intent::Login);
        assert_eq!(h.raw, bytes);
        // Nothing after the frame was consumed.
        assert_eq!(r, b"login start");
    }

    #[test]
    fn a_vanilla_handshake_byte_for_byte() {
        // A 26.3 client's status handshake for "a.b:25565" as on the wire:
        // length 10, id 0, protocol 777 as a VarInt (0x89 0x06), the
        // address with its length, port 25565 big-endian, intent 1.
        let wire = [10, 0, 0x89, 0x06, 3, b'a', b'.', b'b', 0x63, 0xdd, 1];
        let h = read(&mut &wire[..]).unwrap();
        assert_eq!(
            (h.protocol, h.address.as_str(), h.port),
            (777, "a.b", 25565)
        );
        assert_eq!(h.intent, Intent::Status);
        assert_eq!(encode(777, "a.b", 25565, Intent::Status), wire);
    }

    #[test]
    fn refuses_what_is_not_a_handshake() {
        // Legacy server list ping.
        assert!(read(&mut &[0xfe, 0x01, 0xfa][..]).is_err());
        // HTTP.
        assert!(read(&mut &b"GET / HTTP/1.1\r\n"[..]).is_err());
        // Oversized length.
        assert!(read(&mut &[0xff, 0xff, 0x03][..]).is_err());
        // Wrong packet id.
        assert!(read(&mut &frame(5, b"\x01\x00\x00\x00\x01")[..]).is_err());
        // Truncated.
        let full = encode(777, "x.example", 25565, Intent::Login);
        assert!(read(&mut &full[..full.len() - 1][..]).is_err());
    }

    #[test]
    fn routes_one_label_under_the_domain() {
        let d = "mc.example.net";
        assert_eq!(
            route(&normalize("PullMon.mc.example.net."), d),
            Some("pullmon")
        );
        assert_eq!(
            route(&normalize("pullmon.mc.example.net\0FML3\0"), d),
            Some("pullmon")
        );
        assert_eq!(route("mc.example.net", d), None);
        assert_eq!(route("a.b.mc.example.net", d), None);
        assert_eq!(route("pullmonmc.example.net", d), None);
        assert_eq!(route("pullmon.example.net", d), None);
        assert_eq!(route("-bad.mc.example.net", d), None);
    }

    #[test]
    fn status_and_disconnect_answers() {
        struct Duplex {
            input: io::Cursor<Vec<u8>>,
            output: Vec<u8>,
        }
        impl Read for Duplex {
            fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
                self.input.read(b)
            }
        }
        impl Write for Duplex {
            fn write(&mut self, b: &[u8]) -> io::Result<usize> {
                self.output.write(b)
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let mut input = frame(0, &[]);
        input.extend(frame(1, &42i64.to_be_bytes()));
        let mut s = Duplex {
            input: io::Cursor::new(input),
            output: Vec::new(),
        };
        answer_status(&mut s, 777, "offline").unwrap();
        let mut out = &s.output[..];
        let status = read_frame(&mut out, 4096).unwrap();
        assert_eq!(status[0], 0);
        let mut at = 1;
        let n = varint_at(&status, &mut at).unwrap() as usize;
        let json: serde_json::Value = serde_json::from_slice(&status[at..at + n]).unwrap();
        assert_eq!(json["description"]["text"], "offline");
        assert_eq!(json["version"]["protocol"], 777);
        assert_eq!(read_frame(&mut out, 9).unwrap()[1..], 42i64.to_be_bytes());

        let mut w = Vec::new();
        refuse_login(&mut w, "no").unwrap();
        let body = read_frame(&mut &w[..], 4096).unwrap();
        assert_eq!(body[0], 0);
        assert_eq!(&body[2..], br#"{"text":"no"}"#);
    }
}
