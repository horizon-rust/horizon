//! Fixed private program-message channels on the existing play connection.
use crate::{
    Error, Result,
    buf::{Reader, Writer},
};
pub const SUBSCRIBE_CHANNEL: &str = "horizon:program_subscriptions_v1";
pub const RESET_CHANNEL: &str = "horizon:program_messages_reset_v1";
pub const DELIVERY_CHANNEL: &str = "horizon:program_messages_v1";
pub const MAX_PAYLOAD: usize = 32768;
pub const MAX_PACKET: usize = MAX_PAYLOAD + 1024;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Subscription {
    pub receiver: String,
    pub epoch: u64,
    pub sender: String,
    pub channel: String,
}
#[derive(Clone, Debug)]
pub struct Delivery {
    pub receiver: String,
    pub epoch: u64,
    pub sender: String,
    pub channel: String,
    pub payload: Vec<u8>,
}
fn owner(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_.-".contains(&c))
}
fn channel(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_.:/-".contains(&c))
}
fn checked(s: &Subscription) -> bool {
    owner(&s.receiver) && owner(&s.sender) && channel(&s.channel) && s.epoch != 0
}
pub fn encode_subscriptions(values: &[Subscription]) -> Vec<u8> {
    let mut w = Writer::new();
    w.u8(1);
    w.varint(values.len() as i32);
    for s in values {
        w.string(&s.receiver);
        w.i64(s.epoch as i64);
        w.string(&s.sender);
        w.string(&s.channel);
    }
    w.data
}
pub fn decode_subscriptions(bytes: &[u8]) -> Result<Vec<Subscription>> {
    if bytes.len() > 32767 {
        return Err(Error::new("program subscriptions too large"));
    }
    let mut r = Reader::new(bytes);
    if r.u8()? != 1 {
        return Err(Error::new("unsupported program subscriptions"));
    }
    let count = r.varint()?;
    if !(0..=64).contains(&count) {
        return Err(Error::new("too many program subscriptions"));
    }
    let mut output = Vec::new();
    for _ in 0..count {
        let s = Subscription {
            receiver: r.string(64)?,
            epoch: r.i64()? as u64,
            sender: r.string(64)?,
            channel: r.string(128)?,
        };
        if !checked(&s) || output.contains(&s) {
            return Err(Error::new("invalid program subscription"));
        }
        output.push(s);
    }
    if r.remaining() != 0 {
        return Err(Error::new("program subscriptions trailing bytes"));
    }
    Ok(output)
}
pub fn encode_delivery(value: &Delivery) -> Vec<u8> {
    let mut w = Writer::new();
    w.u8(1);
    w.string(&value.receiver);
    w.i64(value.epoch as i64);
    w.string(&value.sender);
    w.string(&value.channel);
    w.byte_array(&value.payload);
    w.data
}
pub fn decode_delivery(bytes: &[u8]) -> Result<Delivery> {
    if bytes.len() > MAX_PACKET {
        return Err(Error::new("program message packet too large"));
    }
    let mut r = Reader::new(bytes);
    if r.u8()? != 1 {
        return Err(Error::new("unsupported program message"));
    }
    let value = Delivery {
        receiver: r.string(64)?,
        epoch: r.i64()? as u64,
        sender: r.string(64)?,
        channel: r.string(128)?,
        payload: r.byte_array(MAX_PAYLOAD)?,
    };
    if !checked(&Subscription {
        receiver: value.receiver.clone(),
        epoch: value.epoch,
        sender: value.sender.clone(),
        channel: value.channel.clone(),
    }) || r.remaining() != 0
    {
        return Err(Error::new("invalid program message"));
    }
    Ok(value)
}
