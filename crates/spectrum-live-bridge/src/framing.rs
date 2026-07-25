use std::{
    collections::BTreeMap,
    io::{Read, Write},
};

use serde::{
    Deserialize, Deserializer, Serialize,
    de::{DeserializeOwned, Error as _, MapAccess, SeqAccess, Visitor},
};
use serde_json::{Number, Value};

use crate::{BridgeError, BridgeResult, MAX_FRAME_BYTES, validate_json_limits};

pub fn write_frame<W: Write, T: Serialize>(writer: &mut W, value: &T) -> BridgeResult<()> {
    let body = serde_json::to_vec(value)?;
    if body.len() > MAX_FRAME_BYTES {
        return Err(BridgeError::Limit("frame exceeds 8 MiB".into()));
    }
    let length = u32::try_from(body.len())
        .map_err(|_| BridgeError::Limit("frame cannot be represented".into()))?;
    writer.write_all(&length.to_be_bytes())?;
    writer.write_all(&body)?;
    writer.flush()?;
    Ok(())
}

pub fn read_frame<R: Read, T: DeserializeOwned>(reader: &mut R) -> BridgeResult<T> {
    FrameReader::new(reader).read()
}

pub struct FrameReader<R> {
    inner: R,
    body: Vec<u8>,
}

impl<R: Read> FrameReader<R> {
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            body: Vec::new(),
        }
    }

    pub fn read<T: DeserializeOwned>(&mut self) -> BridgeResult<T> {
        let mut prefix = [0_u8; 4];
        self.inner.read_exact(&mut prefix)?;
        let length = u32::from_be_bytes(prefix) as usize;
        if length == 0 {
            return Err(BridgeError::Protocol("empty frame".into()));
        }
        if length > MAX_FRAME_BYTES {
            return Err(BridgeError::Limit("frame exceeds 8 MiB".into()));
        }
        self.body.resize(length, 0);
        self.inner.read_exact(&mut self.body)?;
        let value = parse_strict_value(&self.body)?;
        validate_json_limits(&value)?;
        Ok(serde_json::from_value(value)?)
    }

    pub fn into_inner(self) -> R {
        self.inner
    }
}

fn parse_strict_value(bytes: &[u8]) -> BridgeResult<Value> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let value = StrictValue::deserialize(&mut deserializer)?.0;
    deserializer.end()?;
    Ok(value)
}

struct StrictValue(Value);

impl<'de> Deserialize<'de> for StrictValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(StrictValueVisitor)
    }
}

struct StrictValueVisitor;

impl<'de> Visitor<'de> for StrictValueVisitor {
    type Value = StrictValue;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a JSON value without duplicate object keys")
    }

    fn visit_bool<E: serde::de::Error>(self, value: bool) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Bool(value)))
    }

    fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Number(value.into())))
    }

    fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Number(value.into())))
    }

    fn visit_f64<E: serde::de::Error>(self, value: f64) -> Result<Self::Value, E> {
        Number::from_f64(value)
            .map(Value::Number)
            .map(StrictValue)
            .ok_or_else(|| E::custom("non-finite JSON number"))
    }

    fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::String(value.to_owned())))
    }

    fn visit_string<E: serde::de::Error>(self, value: String) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::String(value)))
    }

    fn visit_none<E: serde::de::Error>(self) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Null))
    }

    fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Null))
    }

    fn visit_some<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        StrictValue::deserialize(deserializer)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element::<StrictValue>()? {
            values.push(value.0);
        }
        Ok(StrictValue(Value::Array(values)))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut fields = BTreeMap::new();
        while let Some((key, value)) = map.next_entry::<String, StrictValue>()? {
            if fields.insert(key.clone(), value.0).is_some() {
                return Err(A::Error::custom(format!("duplicate object key `{key}`")));
            }
        }
        Ok(StrictValue(Value::Object(fields.into_iter().collect())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_round_trip_and_strict_duplicate_rejection() {
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &serde_json::json!({"ok": true})).unwrap();
        let decoded: Value = read_frame(&mut bytes.as_slice()).unwrap();
        assert_eq!(decoded, serde_json::json!({"ok": true}));

        let body = br#"{"x":1,"x":2}"#;
        let mut duplicate = Vec::from((body.len() as u32).to_be_bytes());
        duplicate.extend_from_slice(body);
        let error = read_frame::<_, Value>(&mut duplicate.as_slice()).unwrap_err();
        assert!(error.to_string().contains("duplicate"));
    }

    #[test]
    fn truncated_and_oversized_frames_fail_closed() {
        let mut truncated = [0, 0, 0, 3, b'{'].as_slice();
        assert!(read_frame::<_, Value>(&mut truncated).is_err());
        let oversized_prefix = ((MAX_FRAME_BYTES as u32) + 1).to_be_bytes();
        let mut oversized = oversized_prefix.as_slice();
        assert!(matches!(
            read_frame::<_, Value>(&mut oversized),
            Err(BridgeError::Limit(_))
        ));
    }

    #[test]
    fn invalid_utf8_and_excessive_depth_fail_closed() {
        let invalid = vec![0, 0, 0, 3, b'"', 0xff, b'"'];
        assert!(read_frame::<_, Value>(&mut invalid.as_slice()).is_err());

        let body = format!(
            "{}0{}",
            "[".repeat(crate::MAX_JSON_DEPTH + 2),
            "]".repeat(crate::MAX_JSON_DEPTH + 2)
        );
        let mut nested = Vec::from((body.len() as u32).to_be_bytes());
        nested.extend_from_slice(body.as_bytes());
        assert!(read_frame::<_, Value>(&mut nested.as_slice()).is_err());
    }
}
