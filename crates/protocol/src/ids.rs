//! Identifiers and revision counters.
//!
//! Wire counters are decimal strings so clients (e.g. JavaScript) never rely on
//! integer precision. `entity_id` is an opaque string
//! that is stable within a single snapshot only.

use serde::{Deserialize, Serialize};

/// Opaque identifier issued per host session.
pub type SessionId = uuid::Uuid;
/// Identifier issued per successful load; a reload issues a new one.
pub type SceneId = uuid::Uuid;
/// Identifier for a running/finished job.
pub type JobId = uuid::Uuid;
/// Identifier for a produced artifact (image, blob, export).
pub type ArtifactId = uuid::Uuid;

/// Opaque, snapshot-scoped entity identifier. Not a UUID: it is derived from
/// (scene_id, kind, ordinal) and must not be reused across snapshots.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EntityId(pub String);

impl EntityId {
    pub fn new(opaque: impl Into<String>) -> Self {
        EntityId(opaque.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A monotonic revision counter. Represented as a decimal string on the wire.
/// A missing/initial revision is `"0"`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct Revision(pub String);

impl TryFrom<String> for Revision {
    type Error = &'static str;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let revision = Self(value);
        revision
            .to_u64()
            .ok_or("expected a canonical u64 decimal string")?;
        Ok(revision)
    }
}

impl Revision {
    pub const INITIAL: &str = "0";

    pub fn initial() -> Self {
        Revision(Revision::INITIAL.to_string())
    }

    pub fn from_u64(v: u64) -> Self {
        Revision(v.to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Parse to u64 for core-side ordering. Returns None for non-numeric input.
    pub fn to_u64(&self) -> Option<u64> {
        let value = self.0.parse::<u64>().ok()?;
        (value.to_string() == self.0).then_some(value)
    }
}

/// Reference to a specific scene snapshot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneRef {
    pub scene_id: SceneId,
    pub scene_revision: Revision,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revision_roundtrips_as_decimal_string() {
        let r = Revision::from_u64(42);
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(json, "\"42\"");
        let back: Revision = serde_json::from_str(&json).unwrap();
        assert_eq!(back, r);
        assert_eq!(back.to_u64(), Some(42));
    }

    #[test]
    fn initial_revision_is_zero() {
        assert_eq!(Revision::initial().as_str(), "0");
    }
}
