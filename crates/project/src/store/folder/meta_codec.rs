use super::*;
use crate::meta_v2::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyMeta {
    format: String,
    id: uuid::Uuid,
    kind: String,
    media_type: Option<String>,
    sha256: Option<String>,
    size: Option<u64>,
    origin: Option<AssetMetaOrigin>,
}

impl<'de> Deserialize<'de> for AssetMeta {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        if value.get("format").and_then(|v| v.as_str()) == Some("pbrt-ui.meta/1") {
            let raw: LegacyMeta =
                serde_json::from_value(value).map_err(serde::de::Error::custom)?;
            return Ok(Self {
                format: raw.format,
                id: raw.id,
                kind: raw.kind,
                media_type: raw.media_type,
                sha256: raw.sha256,
                size: raw.size,
                origin: raw.origin,
            });
        }
        let meta: MetaV2 = serde_json::from_value(value).map_err(serde::de::Error::custom)?;
        meta.validate().map_err(serde::de::Error::custom)?;
        let mut result = Self {
            format: "pbrt-ui.meta/2".into(),
            id: uuid::Uuid::nil(),
            kind: String::new(),
            media_type: None,
            sha256: None,
            size: None,
            origin: None,
        };
        match meta {
            MetaV2::Scene { id, .. } => {
                result.id = id.0;
                result.kind = "scene".into();
            }
            MetaV2::Resource { id, .. } => {
                result.id = id.0;
                result.kind = "resource".into();
            }
            MetaV2::Asset {
                id,
                media_type,
                sha256,
                size,
                origin,
                ..
            } => {
                result.id = id.0;
                result.kind = "asset".into();
                result.media_type = Some(media_type);
                result.sha256 = Some(sha256);
                result.size = Some(size);
                result.origin = origin.map(legacy_origin);
            }
        }
        Ok(result)
    }
}

fn legacy_origin(origin: StoredAssetOrigin) -> AssetMetaOrigin {
    AssetMetaOrigin {
        import: origin.import,
        logical_path: origin.logical_path,
    }
}

impl AssetMeta {
    pub(super) fn into_v2(self) -> Result<MetaV2> {
        let format = MetaVersion::V2;
        let meta = match self.kind.as_str() {
            "scene" => MetaV2::Scene {
                format,
                id: SceneId(self.id),
            },
            "resource" => MetaV2::Resource {
                format,
                id: ResourceId(self.id),
            },
            "asset" => MetaV2::Asset {
                format,
                id: AssetId(self.id),
                media_type: self
                    .media_type
                    .ok_or_else(|| invalid("asset media type missing"))?,
                sha256: self.sha256.ok_or_else(|| invalid("asset hash missing"))?,
                size: self.size.ok_or_else(|| invalid("asset size missing"))?,
                origin: self.origin.map(|origin| StoredAssetOrigin {
                    import: origin.import,
                    logical_path: origin.logical_path,
                }),
            },
            _ => return Err(invalid("unknown sidecar kind")),
        };
        meta.validate()?;
        Ok(meta)
    }
}
