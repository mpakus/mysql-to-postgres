//! Validate JSON strings/keys while discarding entries, without building a DOM.
use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use std::fmt;

#[derive(Clone, Copy)]
struct Discard {
    token_capacity: usize,
}
impl<'de> DeserializeSeed<'de> for Discard {
    type Value = ();
    fn deserialize<D: de::Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        deserializer.deserialize_any(self)
    }
}
impl<'de> Visitor<'de> for Discard {
    type Value = ();
    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("JSON without decoded NUL")
    }
    fn visit_unit<E: de::Error>(self) -> Result<(), E> {
        Ok(())
    }
    fn visit_bool<E: de::Error>(self, _: bool) -> Result<(), E> {
        Ok(())
    }
    fn visit_i64<E: de::Error>(self, _: i64) -> Result<(), E> {
        Ok(())
    }
    fn visit_u64<E: de::Error>(self, _: u64) -> Result<(), E> {
        Ok(())
    }
    fn visit_f64<E: de::Error>(self, _: f64) -> Result<(), E> {
        Ok(())
    }
    fn visit_str<E: de::Error>(self, value: &str) -> Result<(), E> {
        if value.contains('\0') {
            Err(E::custom("decoded NUL is not representable by jsonb"))
        } else {
            Ok(())
        }
    }
    fn visit_string<E: de::Error>(self, value: String) -> Result<(), E> {
        if value.capacity() > self.token_capacity {
            return Err(E::custom(
                "JSON decoder token exceeded the pinned capacity contract",
            ));
        }
        self.visit_str(&value)
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<(), A::Error> {
        while sequence.next_element_seed(self)?.is_some() {}
        Ok(())
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        while map.next_key_seed(self)?.is_some() {
            map.next_value_seed(self)?;
        }
        Ok(())
    }
}

pub(super) fn validate(text: &str) -> Result<(), serde_json::Error> {
    let mut deserializer = serde_json::Deserializer::from_str(text);
    Discard {
        token_capacity: text.len().saturating_mul(2).max(16),
    }
    .deserialize(&mut deserializer)?;
    deserializer.end()?;
    numeric_ranges(text).map_err(<serde_json::Error as de::Error>::custom)
}

// Syntax has already been accepted by serde_json. This scans only lexemes outside
// strings to apply the jsonb/numeric domain; no grammar, decoding or allocation.
fn numeric_ranges(text: &str) -> Result<(), &'static str> {
    let bytes = text.as_bytes();
    let mut index = 0;
    let mut quoted = false;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' if quoted => {
                index += 2;
                continue;
            }
            b'"' => quoted = !quoted,
            b'-' | b'0'..=b'9' if !quoted => {
                let start = index;
                index += 1;
                while index < bytes.len()
                    && matches!(bytes[index], b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-')
                {
                    index += 1;
                }
                numeric_range(&text[start..index])?;
                continue;
            }
            _ => {}
        }
        index += 1;
    }
    Ok(())
}
fn numeric_range(text: &str) -> Result<(), &'static str> {
    let invalid = "JSON number exceeds PostgreSQL numeric range";
    let unsigned = text.strip_prefix('-').unwrap_or(text);
    let (mantissa, exponent) = unsigned.split_once(['e', 'E']).unwrap_or((unsigned, "0"));
    let exponent = exponent.parse::<i64>().map_err(|_| invalid)?;
    let (integral, fractional) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let leading = integral
        .bytes()
        .chain(fractional.bytes())
        .take_while(|byte| *byte == b'0')
        .count();
    let before = (integral.len() as i64)
        .checked_add(exponent)
        .and_then(|value| value.checked_sub(leading as i64))
        .ok_or(invalid)?;
    let scale = (fractional.len() as i64)
        .checked_sub(exponent)
        .ok_or(invalid)?;
    if before > 131072 || scale > 16383 {
        Err(invalid)
    } else {
        Ok(())
    }
}
