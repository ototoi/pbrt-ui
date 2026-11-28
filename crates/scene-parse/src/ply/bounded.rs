//! Streaming validation for imported meshes; no vertex/face-sized allocations.
use super::*;
use std::{
    cell::Cell,
    collections::BTreeSet,
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    path::Path,
    rc::Rc,
};

#[derive(Clone, Copy, Debug)]
pub struct PlyValidationLimits {
    pub header_bytes: u64,
    pub properties: usize,
    pub vertices: usize,
    pub faces: usize,
    pub indices: usize,
    pub triangles: usize,
}
impl Default for PlyValidationLimits {
    fn default() -> Self {
        Self {
            header_bytes: 1024 * 1024,
            properties: 256,
            vertices: 10_000_000,
            faces: 10_000_000,
            indices: 30_000_000,
            triangles: 10_000_000,
        }
    }
}
#[derive(Debug, thiserror::Error)]
pub enum PlyValidationError {
    #[error(transparent)]
    Invalid(#[from] SceneParseError),
    #[error("PLY limit exceeded: {0}")]
    Limit(&'static str),
    #[error("PLY validation cancelled")]
    Cancelled,
}

type Result<T> = std::result::Result<T, PlyValidationError>;
const MAX_EXPANDED_GZIP_PLY_BYTES: u64 = 1024 * 1024 * 1024;

pub fn validate_ply_file(
    path: &Path,
    limits: PlyValidationLimits,
    cancelled: impl Fn() -> bool,
) -> Result<PlyInfo> {
    let name = path.to_string_lossy();
    let file =
        std::fs::File::open(path).map_err(|e| PlyValidationError::Invalid(error(&name, e)))?;
    let mut file = file;
    let mut magic = [0; 2];
    let is_gzip = file.read_exact(&mut magic).is_ok() && magic == [0x1f, 0x8b];
    file.seek(SeekFrom::Start(0))
        .map_err(|e| PlyValidationError::Invalid(error(&name, e)))?;
    if is_gzip {
        validate_ply_gzip_reader(&name, file, limits, MAX_EXPANDED_GZIP_PLY_BYTES, cancelled)
    } else {
        validate_ply_reader(&name, file, limits, cancelled, false)
    }
}

pub fn validate_ply_gzip_file(
    path: &Path,
    limits: PlyValidationLimits,
    cancelled: impl Fn() -> bool,
) -> Result<PlyInfo> {
    let name = path.to_string_lossy();
    let file =
        std::fs::File::open(path).map_err(|e| PlyValidationError::Invalid(error(&name, e)))?;
    validate_ply_gzip_reader(&name, file, limits, MAX_EXPANDED_GZIP_PLY_BYTES, cancelled)
}

fn validate_ply_gzip_reader(
    name: &str,
    file: std::fs::File,
    limits: PlyValidationLimits,
    expanded_limit: u64,
    cancelled: impl Fn() -> bool,
) -> Result<PlyInfo> {
    let exceeded = Rc::new(Cell::new(false));
    let input = ExpandedLimitReader {
        input: flate2::read::MultiGzDecoder::new(file),
        read: 0,
        limit: expanded_limit,
        exceeded: Rc::clone(&exceeded),
    };
    let result = validate_ply_reader(name, input, limits, cancelled, true);
    if exceeded.get() {
        Err(PlyValidationError::Limit("expanded bytes"))
    } else {
        result
    }
}

fn validate_ply_reader(
    name: &str,
    input: impl Read,
    limits: PlyValidationLimits,
    cancelled: impl Fn() -> bool,
    drain_to_end: bool,
) -> Result<PlyInfo> {
    let fail = |message: &str| PlyValidationError::Invalid(error(name, message));
    if cancelled() {
        return Err(PlyValidationError::Cancelled);
    }
    let mut reader = BufReader::new(input);
    let mut bytes = Vec::new();
    {
        let mut bounded = reader.by_ref().take(limits.header_bytes.saturating_add(1));
        loop {
            let read = bounded
                .read_until(b'\n', &mut bytes)
                .map_err(|e| fail(&e.to_string()))?;
            if bytes.len() as u64 > limits.header_bytes {
                return Err(PlyValidationError::Limit("header bytes"));
            }
            if bytes.ends_with(b"end_header\n") || bytes.ends_with(b"end_header\r\n") {
                break;
            }
            if read == 0 {
                return Err(fail("missing end_header"));
            }
        }
    }
    // The rendering reader supports scalar vertices followed by a single
    // vertex-index face list. Reject unsupported layouts instead of validating
    // one interpretation and later rendering another.
    let text = std::str::from_utf8(&bytes).map_err(|e| fail(&e.to_string()))?;
    let mut elements = BTreeSet::new();
    let mut element = "";
    let mut properties = 0;
    for line in text.lines() {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        match fields.as_slice() {
            ["ply"] | ["end_header"] => {}
            [
                "format",
                "ascii" | "binary_little_endian" | "binary_big_endian",
                "1.0",
            ] => {}
            ["comment" | "obj_info", ..] => {}
            ["element", name @ ("vertex" | "face"), count] => {
                if !elements.insert(*name) || (*name == "face" && !elements.contains("vertex")) {
                    return Err(fail("unsupported element order"));
                }
                let count = count
                    .parse::<usize>()
                    .map_err(|_| fail("invalid element count"))?;
                if (*name == "vertex" && count > limits.vertices)
                    || (*name == "face" && count > limits.faces)
                {
                    return Err(PlyValidationError::Limit("element count"));
                }
                element = name;
                properties = 0;
            }
            ["property", ty, _] if element == "vertex" => {
                scalar_size(ty)?;
                properties += 1;
                if properties > limits.properties {
                    return Err(PlyValidationError::Limit("properties"));
                }
            }
            [
                "property",
                "list",
                count,
                item,
                "vertex_indices" | "vertex_index",
            ] if element == "face" && properties == 0 => {
                for ty in [count, item] {
                    scalar_size(ty)?;
                    if matches!(*ty, "float" | "float32" | "double" | "float64") {
                        return Err(fail("face lists require integer types"));
                    }
                }
                properties += 1;
            }
            _ => return Err(fail("unsupported PLY header layout")),
        }
    }
    let header = parse_header(name, &bytes)?;
    if header.vertices == 0 || header.faces == 0 {
        return Err(fail("PLY contains no triangle mesh"));
    }
    let mut indices = 0usize;
    let mut triangles = 0usize;
    let little = header.format == "binary_little_endian";
    for _ in 0..header.vertices {
        if cancelled() {
            return Err(PlyValidationError::Cancelled);
        }
        for property in &header.vertex_props {
            let Property::Scalar(ty, _) = property;
            let value = scalar(&mut reader, &header.format, ty, little, name, false)?;
            if !value.is_finite() {
                return Err(fail("nonfinite vertex value"));
            }
        }
    }
    let (count_type, index_type, _) = header
        .face_prop
        .as_ref()
        .ok_or_else(|| fail("missing face indices"))?;
    for _ in 0..header.faces {
        if cancelled() {
            return Err(PlyValidationError::Cancelled);
        }
        // Face tokens use the mesh reader's strict integer syntax: the
        // values read_faces_ascii parses as usize/u32, not the integral
        // floating notation a zero-fraction f64 would still let through.
        let count = scalar(&mut reader, &header.format, count_type, little, name, true)?;
        if !count.is_finite() || !(3. ..=1_000_000.).contains(&count) {
            return Err(fail("invalid face index count"));
        }
        let count = count as usize;
        indices = indices
            .checked_add(count)
            .filter(|count| *count <= limits.indices)
            .ok_or(PlyValidationError::Limit("face indices"))?;
        triangles = triangles
            .checked_add(count - 2)
            .filter(|count| *count <= limits.triangles)
            .ok_or(PlyValidationError::Limit("triangles"))?;
        for index in 0..count {
            if index % 4096 == 0 && cancelled() {
                return Err(PlyValidationError::Cancelled);
            }
            let value = scalar(&mut reader, &header.format, index_type, little, name, true)?;
            if !value.is_finite()
                || value < 0.
                || value >= header.vertices as f64
                || value.fract() != 0.
            {
                return Err(fail("face index outside vertex array"));
            }
        }
    }
    if drain_to_end {
        let mut tail = [0; 64 * 1024];
        loop {
            if cancelled() {
                return Err(PlyValidationError::Cancelled);
            }
            if reader.read(&mut tail).map_err(|e| fail(&e.to_string()))? == 0 {
                break;
            }
        }
    }
    Ok(ply_info(name, &bytes)?)
}

struct ExpandedLimitReader<R> {
    input: R,
    read: u64,
    limit: u64,
    exceeded: Rc<Cell<bool>>,
}

impl<R: Read> Read for ExpandedLimitReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        if self.read == self.limit {
            let mut probe = [0; 1];
            if self.input.read(&mut probe)? != 0 {
                self.exceeded.set(true);
            }
            return Ok(0);
        }
        let remaining = self.limit - self.read;
        let read_limit = buffer.len().min(remaining as usize);
        let count = self.input.read(&mut buffer[..read_limit])?;
        self.read += count as u64;
        Ok(count)
    }
}

fn scalar(
    reader: &mut impl BufRead,
    format: &str,
    ty: &str,
    little: bool,
    path: &str,
    face_token: bool,
) -> Result<f64> {
    let invalid = |message: &str| PlyValidationError::Invalid(error(path, message));
    if format == "ascii" {
        // Fixed stack storage: one small Vec per token dominated ASCII scans
        // of large meshes, and 128 bytes bounds every accepted token anyway.
        let mut token = [0u8; 128];
        let mut length = 0usize;
        loop {
            let available = reader.fill_buf().map_err(|e| invalid(&e.to_string()))?;
            if available.is_empty() {
                if length == 0 {
                    return Err(invalid("truncated PLY body"));
                }
                break;
            }
            let byte = available[0];
            reader.consume(1);
            if byte.is_ascii_whitespace() {
                if length > 0 {
                    break;
                }
            } else {
                if length == 128 {
                    return Err(PlyValidationError::Limit("scalar token bytes"));
                }
                token[length] = byte;
                length += 1;
            }
        }
        let text = std::str::from_utf8(&token[..length]).map_err(|e| invalid(&e.to_string()))?;
        if face_token {
            // Face counts and indices are read as usize/u32 when the mesh
            // loads, so the token must be integer syntax — "3.0" parses as
            // an integral f64 here but the load would reject the file.
            let digits = text.strip_prefix('+').unwrap_or(text);
            if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(invalid("face tokens require integer syntax"));
            }
        }
        let value = text.parse::<f64>().map_err(|e| invalid(&e.to_string()))?;
        if !matches!(ty, "float" | "float32" | "double" | "float64") && value.fract() != 0. {
            return Err(invalid("nonintegral integer property"));
        }
        Ok(value)
    } else {
        let length = scalar_size(ty)?;
        let mut bytes = [0u8; 8];
        reader
            .read_exact(&mut bytes[..length])
            .map_err(|e| invalid(&e.to_string()))?;
        decode_scalar(ty, &bytes[..length], little).map_err(|_| invalid("invalid scalar"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    const TRIANGLE: &[u8] = b"ply\nformat ascii 1.0\nelement vertex 3\nproperty float x\nproperty float y\nproperty float z\nelement face 1\nproperty list uchar int vertex_indices\nend_header\n0 0 0\n1 0 0\n0 1 0\n3 0 1 2\n";

    fn compressed(ply: &[u8]) -> Vec<u8> {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(ply).unwrap();
        encoder.finish().unwrap()
    }

    fn file(bytes: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mesh.ply.gz");
        std::fs::write(&path, bytes).unwrap();
        (dir, path)
    }

    #[test]
    fn gzip_validation_checks_crc_and_trailer_even_when_ply_body_is_complete() {
        let mut bad_crc = compressed(TRIANGLE);
        let crc_start = bad_crc.len() - 8;
        bad_crc[crc_start] ^= 0x80;
        let (_dir, path) = file(&bad_crc);
        assert!(matches!(
            validate_ply_gzip_file(&path, PlyValidationLimits::default(), || false),
            Err(PlyValidationError::Invalid(_))
        ));

        let mut truncated = compressed(TRIANGLE);
        truncated.truncate(truncated.len() - 4);
        let (_dir, path) = file(&truncated);
        assert!(matches!(
            validate_ply_gzip_file(&path, PlyValidationLimits::default(), || false),
            Err(PlyValidationError::Invalid(_))
        ));
    }

    #[test]
    fn expanded_gzip_limit_accepts_exact_body_and_rejects_one_byte_less() {
        let (_dir, path) = file(&compressed(TRIANGLE));
        validate_ply_gzip_reader(
            &path.to_string_lossy(),
            std::fs::File::open(&path).unwrap(),
            PlyValidationLimits::default(),
            TRIANGLE.len() as u64,
            || false,
        )
        .unwrap();

        assert!(matches!(
            validate_ply_gzip_reader(
                &path.to_string_lossy(),
                std::fs::File::open(&path).unwrap(),
                PlyValidationLimits::default(),
                TRIANGLE.len() as u64 - 1,
                || false,
            ),
            Err(PlyValidationError::Limit("expanded bytes"))
        ));
    }
}
