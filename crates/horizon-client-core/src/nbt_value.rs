//! Reading and writing single NBT values the way vanilla's codecs accept them.
use horizon_world::nbt::Tag;

pub fn string(s: &str) -> Tag {
    Tag::String(s.into())
}
pub fn text(tag: Option<&Tag>) -> Option<String> {
    match tag {
        Some(Tag::String(s)) => Some(s.to_string_lossy()),
        _ => None,
    }
}
/// Any numeric tag as i64 (vanilla's `getIntOr` etc. accept every number type).
pub fn number(tag: Option<&Tag>) -> Option<i64> {
    Some(match tag? {
        Tag::Byte(v) => i64::from(*v),
        Tag::Short(v) => i64::from(*v),
        Tag::Int(v) => i64::from(*v),
        Tag::Long(v) => *v,
        Tag::Float(b) => f32::from_bits(*b) as i64,
        Tag::Double(b) => f64::from_bits(*b) as i64,
        _ => return None,
    })
}
/// Any numeric tag as f64, keeping a float's or double's fraction.
pub fn float(tag: Option<&Tag>) -> Option<f64> {
    Some(match tag? {
        Tag::Float(b) => f64::from(f32::from_bits(*b)),
        Tag::Double(b) => f64::from_bits(*b),
        other => number(Some(other))? as f64,
    })
}
pub fn doubles(values: &[f64]) -> Tag {
    Tag::List(values.iter().map(|v| Tag::Double(v.to_bits())).collect())
}
pub fn floats(values: &[f32]) -> Tag {
    Tag::List(values.iter().map(|v| Tag::Float(v.to_bits())).collect())
}
pub fn read_doubles<const N: usize>(tag: Option<&Tag>) -> Option<[f64; N]> {
    match tag? {
        Tag::List(l) if l.len() == N => {
            let mut out = [0.0; N];
            for (o, v) in out.iter_mut().zip(l) {
                *o = float(Some(v))?;
            }
            Some(out)
        }
        _ => None,
    }
}
pub fn put(tag: &mut Tag, key: &str, value: Tag) {
    if let Tag::Compound(map) = tag {
        map.insert(key.into(), value);
    }
}
