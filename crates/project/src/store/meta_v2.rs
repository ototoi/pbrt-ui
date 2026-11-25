//! Format-3 sidecars. These are persistent identities and settings, not cache.
//! The version is part of the schema so a future version cannot deserialize
//! successfully through this reader.
use super::*;

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum MetaVersion {
    #[serde(rename = "pbrt-ui.meta/2")]
    V2,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredAssetOrigin {
    pub import: ImportId,
    pub logical_path: String,
}

/// Each variant has its own field set; unknown fields are rejected.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MetaV2 {
    Scene {
        format: MetaVersion,
        id: SceneId,
    },
    Resource {
        format: MetaVersion,
        id: ResourceId,
    },
    Asset {
        format: MetaVersion,
        id: AssetId,
        media_type: String,
        sha256: String,
        size: u64,
        origin: Option<StoredAssetOrigin>,
    },
}

impl MetaV2 {
    /// Decode and validate both shape and local invariants. Cross-item UUID
    /// ownership and reference checks belong to document validation.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > 1024 * 1024 {
            return Err(ProjectError::ResourceLimit("sidecar exceeds 1 MiB".into()));
        }
        let meta: Self = serde_json::from_slice(bytes)?;
        meta.validate()?;
        Ok(meta)
    }

    pub fn validate(&self) -> Result<()> {
        let id = match self {
            Self::Scene { id, .. } => id.0,
            Self::Resource { id, .. } => id.0,
            Self::Asset { id, .. } => id.0,
        };
        if id.is_nil() {
            return Err(invalid("sidecar identity must not be nil"));
        }
        match self {
            Self::Asset {
                media_type,
                sha256,
                size,
                origin,
                ..
            } => {
                validate_payload(media_type, sha256, *size, origin.as_ref())?;
            }
            _ => {}
        }
        Ok(())
    }
}

fn validate_payload(
    media_type: &str,
    sha256: &str,
    size: u64,
    origin: Option<&StoredAssetOrigin>,
) -> Result<()> {
    if media_type.is_empty() || !media_type.contains('/') {
        return Err(invalid("asset media type must be a nonempty MIME type"));
    }
    if sha256.len() != 64
        || !sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(invalid("asset SHA-256 must be 64 lowercase hex digits"));
    }
    if size > MAX_OBJECT_BYTES {
        return Err(ProjectError::ResourceLimit("asset exceeds 4 GiB".into()));
    }
    if let Some(origin) = origin {
        if origin.import.0.is_nil() || origin.logical_path.is_empty() {
            return Err(invalid("asset import origin is incomplete"));
        }
    }
    Ok(())
}
