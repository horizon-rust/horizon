//! Private provisional custom entity transport over the existing play connection.
use crate::{
    Error, Result,
    buf::{Reader, Writer},
};
pub const CHANNEL: &str = "horizon:entity_views_v1";
pub const MAX_PAYLOAD: usize = 16384;
#[derive(Clone, Debug, PartialEq)]
pub struct View {
    pub id: u32,
    pub kind: String,
    pub renderer: String,
    pub public_version: u32,
    pub dimension: String,
    pub position: [f64; 3],
    pub rotation: [f32; 2],
    pub public_state: Vec<u8>,
    pub revision: u64,
    pub range: f32,
    /// Authoritative type width and height. Old view messages omit this.
    pub body: Option<[f32; 2]>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    Upsert { world: u64, view: View },
    Remove { world: u64, id: u32 },
}
pub fn encode(message: &Message) -> Vec<u8> {
    let mut w = Writer::new();
    w.u8(1);
    match message {
        Message::Remove { world, id } => {
            w.u8(0);
            w.i64(*world as i64);
            w.u32(*id);
        }
        Message::Upsert { world, view: v } => {
            w.u8(1);
            w.i64(*world as i64);
            w.u32(v.id);
            w.string(&v.kind);
            w.string(&v.renderer);
            w.u32(v.public_version);
            w.string(&v.dimension);
            for p in v.position {
                w.f64(p);
            }
            for r in v.rotation {
                w.f32(r);
            }
            w.byte_array(&v.public_state);
            w.i64(v.revision as i64);
            w.f32(v.range);
            if let Some(body) = v.body {
                w.f32(body[0]);
                w.f32(body[1]);
            }
        }
    }
    w.data
}
pub fn decode(data: &[u8]) -> Result<Message> {
    if data.len() > MAX_PAYLOAD {
        return Err(Error::new("entity payload too large"));
    }
    let mut r = Reader::new(data);
    if r.u8()? != 1 {
        return Err(Error::new("unsupported entity protocol"));
    }
    let op = r.u8()?;
    let world = r.i64()? as u64;
    let id = r.u32()?;
    if world == 0 || id == 0 || id > i32::MAX as u32 {
        return Err(Error::new("invalid entity identity"));
    }
    let message = match op {
        0 => Message::Remove { world, id },
        1 => {
            let kind = r.string(128)?;
            let renderer = r.string(128)?;
            let public_version = r.u32()?;
            let dimension = r.string(128)?;
            let position = [r.f64()?, r.f64()?, r.f64()?];
            let rotation = [r.f32()?, r.f32()?];
            let public_state = r.byte_array(1024)?;
            let revision = r.i64()? as u64;
            let range = r.f32()?;
            let body = match r.remaining() {
                0 => None,
                8 => {
                    let body = [r.f32()?, r.f32()?];
                    if body.iter().any(|n| !n.is_finite() || *n <= 0. || *n > 64.) {
                        return Err(Error::new("invalid entity body dimensions"));
                    }
                    Some(body)
                }
                _ => return Err(Error::new("invalid optional entity body metadata")),
            };
            if position
                .iter()
                .any(|p| !p.is_finite() || p.abs() > 30_000_000.)
                || rotation.iter().any(|r| !r.is_finite())
                || !range.is_finite()
                || range <= 0.
                || range > 256.
                || !(1..=65535).contains(&public_version)
            {
                return Err(Error::new("invalid entity view"));
            }
            Message::Upsert {
                world,
                view: View {
                    id,
                    kind,
                    renderer,
                    public_version,
                    dimension,
                    position,
                    rotation,
                    public_state,
                    revision,
                    range,
                    body,
                },
            }
        }
        _ => return Err(Error::new("invalid entity operation")),
    };
    if r.remaining() != 0 {
        return Err(Error::new("entity payload trailing bytes"));
    }
    Ok(message)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn view_roundtrip() {
        let m = Message::Upsert {
            world: 1,
            view: View {
                id: 4,
                kind: "lab:entity".into(),
                renderer: "lab:checker".into(),
                public_version: 1,
                dimension: "minecraft:overworld".into(),
                position: [1., 2., 3.],
                rotation: [0., 0.],
                public_state: vec![7; 1024],
                revision: 1,
                range: 64.,
                body: None,
            },
        };
        assert_eq!(decode(&encode(&m)).unwrap(), m);
        assert!(decode(&vec![0; MAX_PAYLOAD + 1]).is_err());
    }
}

pub const RENDERERS_CHANNEL: &str = "horizon:entity_renderers_v1";
pub fn encode_renderers(renderers: &[(String, u32)]) -> Vec<u8> {
    let mut w = Writer::new();
    w.u8(1);
    w.varint(renderers.len() as i32);
    for (key, version) in renderers {
        w.string(key);
        w.u32(*version);
    }
    w.data
}
pub fn decode_renderers(bytes: &[u8]) -> Result<Vec<(String, u32)>> {
    if bytes.len() > MAX_PAYLOAD {
        return Err(Error::new("renderer payload too large"));
    }
    let mut r = Reader::new(bytes);
    if r.u8()? != 1 {
        return Err(Error::new("unsupported renderer protocol"));
    }
    let count = r.varint()?;
    if !(0..=64).contains(&count) {
        return Err(Error::new("too many renderers"));
    }
    let mut keys = Vec::new();
    for _ in 0..count {
        let key = r.string(128)?;
        let version = r.u32()?;
        if !key.contains(':')
            || !(1..=65535).contains(&version)
            || keys.iter().any(|(k, _)| k == &key)
        {
            return Err(Error::new("invalid renderer contract"));
        }
        keys.push((key, version));
    }
    if r.remaining() != 0 {
        return Err(Error::new("renderer payload trailing bytes"));
    }
    Ok(keys)
}
