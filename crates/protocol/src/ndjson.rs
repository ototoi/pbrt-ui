//! Shared v1 framing and strict JSON decoding for socket clients and hosts.
//!
//! The byte limit excludes the LF (or CRLF) terminator. A framing error is
//! terminal for a connection: callers must close it instead of interpreting
//! the unread remainder of an oversized line as another request.

use std::fmt;
use std::io::{self, BufRead, Write};

use serde::de::{DeserializeOwned, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

use crate::wire::{MAX_LINE_BYTES, Request};

#[derive(Debug, thiserror::Error)]
pub enum WireError {
    #[error("NDJSON message exceeds {MAX_LINE_BYTES} bytes")]
    LineTooLong,
    #[error("NDJSON message must end in a newline")]
    UnterminatedLine,
    #[error("NDJSON requires one JSON object on a single line")]
    InvalidFrame,
    #[error("invalid JSON message: {0}")]
    Json(#[from] serde_json::Error),
    #[error("wire I/O failed: {0}")]
    Io(#[from] io::Error),
}

/// Decode one JSON object, optionally followed by one LF or CRLF.
/// Duplicate keys are rejected at every depth, including method parameters.
/// Method-specific parameter validation belongs to the core dispatcher.
pub fn decode<T: DeserializeOwned>(line: &[u8]) -> Result<T, WireError> {
    let payload = line.strip_suffix(b"\n").unwrap_or(line);
    let payload = if line.ends_with(b"\n") {
        payload.strip_suffix(b"\r").unwrap_or(payload)
    } else {
        payload
    };
    if payload.len() > MAX_LINE_BYTES {
        return Err(WireError::LineTooLong);
    }
    if payload.contains(&b'\n') || payload.contains(&b'\r') {
        return Err(WireError::InvalidFrame);
    }
    let StrictValue(value) = serde_json::from_slice(payload)?;
    if !value.is_object() {
        return Err(WireError::InvalidFrame);
    }
    Ok(serde_json::from_value(value)?)
}

/// Read a complete frame with bounded allocation. EOF between frames returns
/// None; EOF inside a frame is an error. Invalid frames are never dispatched.
pub fn read_message<T: DeserializeOwned>(
    reader: &mut impl BufRead,
) -> Result<Option<T>, WireError> {
    let mut line = Vec::new();
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return if line.is_empty() {
                Ok(None)
            } else {
                Err(WireError::UnterminatedLine)
            };
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let count = newline.map_or(available.len(), |index| index + 1);
        // One extra byte permits CR at the payload boundary. LF is consumed
        // separately below; decode checks the exact payload length.
        let content_count = count - usize::from(newline.is_some());
        if line.len() + content_count > MAX_LINE_BYTES + 1 {
            return Err(WireError::LineTooLong);
        }
        line.extend_from_slice(&available[..count]);
        reader.consume(count);
        if newline.is_some() {
            return decode(&line).map(Some);
        }
    }
}

/// Encode and validate a message before writing any bytes. Flush policy is
/// left to the caller so multiple responses can be batched on a connection.
pub fn write_message<T: Serialize + DeserializeOwned>(
    writer: &mut impl Write,
    message: &T,
) -> Result<(), WireError> {
    let mut buffer = LimitedBuffer(Vec::new());
    if let Err(error) = serde_json::to_writer(&mut buffer, message) {
        return if buffer.0.len() == MAX_LINE_BYTES {
            Err(WireError::LineTooLong)
        } else {
            Err(WireError::Json(error))
        };
    }
    // Public DTO fields may have been constructed manually. Check the same
    // invariants as the receiver (notably result/error exclusivity).
    let _: T = decode(&buffer.0)?;
    writer.write_all(&buffer.0)?;
    writer.write_all(b"\n")?;
    Ok(())
}

struct LimitedBuffer(Vec<u8>);

impl Write for LimitedBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > MAX_LINE_BYTES - self.0.len() {
            // Fill only up to the cap, never allocating the oversized value.
            let remaining = MAX_LINE_BYTES - self.0.len();
            self.0.extend_from_slice(&bytes[..remaining]);
            return Err(io::Error::other("NDJSON line limit exceeded"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Stable method/params bytes for session-wide request deduplication. Protocol
/// and request_id are envelope fields and do not affect operation identity.
/// Object keys are sorted by UTF-8 bytes; array order is preserved.
pub fn canonical_operation(request: &Request) -> Result<Vec<u8>, serde_json::Error> {
    canonical_json(&serde_json::json!({ "method": request.method, "params": request.params }))
}

/// Canonical JSON shared by request identity and dependency fingerprints.
pub fn canonical_json(value: &Value) -> Result<Vec<u8>, serde_json::Error> {
    fn sorted(value: &Value) -> Value {
        match value {
            Value::Object(object) => {
                let mut entries: Vec<_> = object.iter().collect();
                entries.sort_unstable_by(|(left, _), (right, _)| {
                    left.as_bytes().cmp(right.as_bytes())
                });
                Value::Object(
                    entries
                        .into_iter()
                        .map(|(key, value)| (key.clone(), sorted(value)))
                        .collect(),
                )
            }
            Value::Array(array) => Value::Array(array.iter().map(sorted).collect()),
            value => value.clone(),
        }
    }
    serde_json::to_vec(&sorted(value))
}

// serde_json::Value normally overwrites duplicate keys. Build values using a
// visitor that checks each key before insertion, including escaped spellings.
struct StrictValue(Value);

impl<'de> Deserialize<'de> for StrictValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct StrictVisitor;

        impl<'de> Visitor<'de> for StrictVisitor {
            type Value = StrictValue;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a JSON value without duplicate object keys")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut object = Map::new();
                while let Some(key) = map.next_key::<String>()? {
                    if object.contains_key(&key) {
                        return Err(serde::de::Error::custom(format!("duplicate key: {key}")));
                    }
                    let StrictValue(value) = map.next_value()?;
                    object.insert(key, value);
                }
                Ok(StrictValue(Value::Object(object)))
            }

            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut sequence: A,
            ) -> Result<Self::Value, A::Error> {
                let mut array = Vec::new();
                while let Some(StrictValue(value)) = sequence.next_element()? {
                    array.push(value);
                }
                Ok(StrictValue(Value::Array(array)))
            }

            fn visit_bool<E: serde::de::Error>(self, value: bool) -> Result<Self::Value, E> {
                Ok(StrictValue(value.into()))
            }

            fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<Self::Value, E> {
                Ok(StrictValue(value.into()))
            }

            fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Self::Value, E> {
                Ok(StrictValue(value.into()))
            }

            fn visit_f64<E: serde::de::Error>(self, value: f64) -> Result<Self::Value, E> {
                serde_json::Number::from_f64(value)
                    .map(|number| StrictValue(Value::Number(number)))
                    .ok_or_else(|| E::custom("non-finite JSON number"))
            }

            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Ok(StrictValue(value.into()))
            }

            fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::Null))
            }
        }

        deserializer.deserialize_any(StrictVisitor)
    }
}
