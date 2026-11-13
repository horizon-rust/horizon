//! Admission-local JsonOps Number conversions. Original decimal lexemes are
//! retained by value::parse_first. Integer conversions truncate decimals and
//! wrap low bits; Gson 2.14's documented artifact limits apply on its decimal
//! fallback (10,000 token characters and absolute scale below 10,000).
use super::Error;
use super::value::Value;

pub(super) fn checked_int(value: &Value) -> Result<i32, Error> {
    int(value).ok_or(if value.is_number() {
        Error::DecodeException
    } else {
        Error::Invalid
    })
}
pub(super) fn checked_long(value: &Value) -> Result<i64, Error> {
    long(value).ok_or(if value.is_number() {
        Error::DecodeException
    } else {
        Error::Invalid
    })
}
pub(super) fn nbt(value: &Value) -> Result<(), Error> {
    // JsonOps.convertTo obtains BigDecimal even when NbtOps ultimately stores
    // an overflowing float. Its decimal parser limits precede that conversion.
    decimal_bits(value.as_number().ok_or(Error::Invalid)?)
        .map(|_| ())
        .ok_or(Error::DecodeException)
}
pub(super) fn java_numeric(value: &Value) -> Result<Value, Error> {
    nbt(value)?;
    let raw = value.as_number().ok_or(Error::Invalid)?;
    if let Some(n) = exact_long(raw) {
        return Ok(Value::Number(n.to_string()));
    }
    let d = raw.parse::<f64>().map_err(|_| Error::DecodeException)?;
    // JsonOps -> JavaOps chooses Float when it exactly represents the
    // rounded double, otherwise Double. JsonWriter rejects non-finite values.
    if !d.is_finite() {
        return Err(Error::DecodeException);
    }
    let s = if f64::from(d as f32) == d {
        horizon_world::snbt_text::java_float(d as f32)
    } else {
        horizon_world::snbt_text::java_double(d)
    };
    Ok(Value::Number(s))
}
pub(super) fn exact_long(raw: &str) -> Option<i64> {
    let (m, e) = match raw.find(['e', 'E']) {
        Some(at) => (&raw[..at], raw[at + 1..].parse::<i64>().ok()?),
        None => (raw, 0),
    };
    let negative = m.starts_with('-');
    let m = m.trim_start_matches('-');
    let fraction = m.find('.').map_or(0, |at| m.len() - at - 1);
    let digits: String = m.chars().filter(|c| *c != '.').collect();
    let scale = e.checked_sub(fraction as i64)?;
    let mut integer = if scale < 0 {
        let end = (digits.len() as i64).checked_add(scale)?;
        if end < 0 {
            return digits.bytes().all(|c| c == b'0').then_some(0);
        }
        let end = end as usize;
        if !digits[end..].bytes().all(|c| c == b'0') {
            return None;
        }
        digits[..end].trim_start_matches('0').to_owned()
    } else {
        let mut s = digits.trim_start_matches('0').to_owned();
        if s.is_empty() {
            return Some(0);
        }
        if s.len() as i64 + scale > 19 {
            return None;
        }
        for _ in 0..scale {
            s.push('0');
        }
        s
    };
    if integer.is_empty() {
        return Some(0);
    }
    if negative {
        integer.insert(0, '-');
    }
    integer.parse().ok()
}

pub(super) fn int(value: &Value) -> Option<i32> {
    let number = value.as_number()?;
    if let Ok(n) = number.parse::<i64>() {
        return Some(n as i32);
    }
    decimal_bits(number).map(|n| n as i32)
}
pub(super) fn long(value: &Value) -> Option<i64> {
    let number = value.as_number()?;
    if let Ok(n) = number.parse::<i64>() {
        return Some(n);
    }
    decimal_bits(number).map(|n| n as i64)
}
fn decimal_bits(number: &str) -> Option<u64> {
    if number.len() > 10000 {
        return None;
    }
    let (mantissa, exponent) = match number.find(['e', 'E']) {
        Some(at) => (&number[..at], number[at + 1..].parse::<i64>().ok()?),
        None => (number, 0),
    };
    let negative = mantissa.starts_with('-');
    let mantissa = mantissa.trim_start_matches('-');
    let fraction = mantissa.find('.').map_or(0, |at| mantissa.len() - at - 1);
    let digits: Vec<_> = mantissa.bytes().filter(|b| *b != b'.').collect();
    let scale = exponent.saturating_sub(fraction as i64);
    if scale.unsigned_abs() >= 10000 {
        return None;
    }
    let end = if scale < 0 {
        (digits.len() as i64).saturating_add(scale).max(0) as usize
    } else {
        digits.len()
    };
    let mut n = 0_u64;
    for digit in &digits[..end] {
        n = n.wrapping_mul(10).wrapping_add(u64::from(*digit - b'0'));
    }
    if scale >= 64 {
        // 10^64 is divisible by 2^64.
        n = 0;
    } else {
        for _ in 0..scale.max(0) {
            n = n.wrapping_mul(10);
        }
    }
    Some(if negative { n.wrapping_neg() } else { n })
}
