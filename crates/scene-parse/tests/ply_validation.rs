//! Imported PLY validation streams data without allocating from untrusted sizes.
//! Every fixture here is tiny; declared sizes and injected budgets are the
//! only thing that varies, so a regression that allocates from a declared
//! count will hang or fail in CI, not pass quietly.
use scene_parse::ply::{
    PlyValidationError as E, PlyValidationError::Cancelled, PlyValidationLimits, validate_ply_file,
};
use std::path::{Path, PathBuf};

struct Directory {
    path: PathBuf,
}
impl Directory {
    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.path.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }
}
fn directory() -> Directory {
    Directory {
        path: tempfile::tempdir().unwrap().keep(),
    }
}

fn validate(path: &Path) -> Result<scene_parse::ply::PlyInfo, E> {
    validate_ply_file(path, PlyValidationLimits::default(), || false)
}
/// Minimal valid triangle PLY in the three formats.
const ASCII_TETRA: &str = "ply\nformat ascii 1.0\nelement vertex 4\nproperty float x\n\
    property float y\nproperty float z\nelement face 4\nproperty list uchar int vertex_indices\n\
    end_header\n0 0 0\n1 0 0\n0 1 0\n0 0 1\n3 0 1 2\n3 0 1 3\n3 1 2 3\n3 2 0 3\n";

fn binary_tetra(little: bool) -> Vec<u8> {
    let mut bytes = format!(
        "ply\nformat {} 1.0\nelement vertex 4\nproperty float x\nproperty float y\n\
         property float z\nelement face 4\nproperty list uchar int vertex_indices\nend_header\n",
        if little {
            "binary_little_endian"
        } else {
            "binary_big_endian"
        }
    )
    .into_bytes();
    for point in [[0f32, 0., 0.], [1., 0., 0.], [0., 1., 0.], [0., 0., 1.]] {
        for value in point {
            bytes.extend(if little {
                value.to_le_bytes()
            } else {
                value.to_be_bytes()
            });
        }
    }
    for face in [[0usize, 1, 2], [0, 3, 1], [1, 3, 2], [2, 3, 0]] {
        bytes.push(3);
        for index in face {
            bytes.extend(if little {
                (index as i32).to_le_bytes()
            } else {
                (index as i32).to_be_bytes()
            });
        }
    }
    bytes
}

#[test]
fn valid_ascii_and_binary_meshes_pass_and_report_header_info() {
    let files = directory();
    let ascii = files.write("a.ply", ASCII_TETRA.as_bytes());
    let little = files.write("b.ply", &binary_tetra(true));
    let big = files.write("c.ply", &binary_tetra(false));
    for (path, format) in [
        (&ascii, "ascii"),
        (&little, "binary_little_endian"),
        (&big, "binary_big_endian"),
    ] {
        let info = validate(path).unwrap();
        assert_eq!(info.vertices, 4);
        assert_eq!(info.faces, 4);
        assert_eq!(info.format, format);
    }
    let unknown = files.write(
        "d.ply",
        b"ply\nformat binary_middle_endian 1.0\nelement vertex 4\nproperty float x\n\
          property float y\nproperty float z\nelement face 4\nend_header\n",
    );
    assert!(matches!(validate(&unknown), Err(E::Invalid(_))));
}

#[test]
fn oversized_declared_counts_are_rejected_in_the_header() {
    let files = directory();
    let header = ASCII_TETRA
        .replace("element vertex 4\n", "element vertex 10000001\n")
        .replace("element face 4\n", "element face 10000001\n");
    let path = files.write("huge.ply", header.as_bytes());
    match validate(&path) {
        Err(E::Limit("element count")) => {}
        other => panic!("vertex/face limits must reject header counts, got {other:?}"),
    }
    let faces_only = ASCII_TETRA.replace("element face 4\n", "element face 10000001\n");
    let path = files.write("huge-faces.ply", faces_only.as_bytes());
    match validate(&path) {
        Err(E::Limit("element count")) => {}
        other => panic!("face limit must reject header counts, got {other:?}"),
    }
}

#[test]
fn header_line_and_property_limits_reject_early() {
    let files = directory();
    // A 2 MiB comment blows the 1 MiB header budget before any body work.
    let mut big = Vec::new();
    big.extend_from_slice(b"ply\nformat ascii 1.0\ncomment ");
    big.extend(std::iter::repeat(b'x').take(2 * 1024 * 1024));
    let over = files.write("big-header.ply", &big);
    match validate(&over) {
        Err(E::Limit("header bytes")) => {}
        other => panic!("expected header budget rejection, got {other:?}"),
    }
    // The boundary needs a valid header that ends precisely at the budget:
    // the exact read passes and one byte less dies at the limit, not as a
    // parse error. (The comment-only file above dies invalid either way.)
    let header_bytes = ASCII_TETRA
        .find("end_header\n")
        .map(|offset| offset + "end_header\n".len())
        .unwrap() as u64;
    let exact = files.write("header-at-limit.ply", ASCII_TETRA.as_bytes());
    validate_ply_file(
        &exact,
        PlyValidationLimits {
            header_bytes,
            ..Default::default()
        },
        || false,
    )
    .unwrap_or_else(|error| panic!("a header exactly at the budget must pass, got {error:?}"));
    let one_less = PlyValidationLimits {
        header_bytes: header_bytes - 1,
        ..Default::default()
    };
    match validate_ply_file(&exact, one_less, || false) {
        Err(E::Limit("header bytes")) => {}
        other => panic!("one byte under a valid header must fail, got {other:?}"),
    }
    let mut properties = String::from("ply\nformat ascii 1.0\nelement vertex 1\n");
    for index in 0..257 {
        properties.push_str(&format!("property float x{index}\n"));
    }
    properties.push_str("element face 1\nproperty list uchar int vertex_indices\nend_header\n");
    let many = files.write("properties.ply", properties.as_bytes());
    match validate(&many) {
        Err(E::Limit("properties")) => {}
        other => panic!("expected property limit, got {other:?}"),
    }
}

#[test]
fn broken_or_unsupported_headers_fail_without_limit_errors() {
    let files = directory();
    let missing_end = files.write("no-end.ply", b"ply\nformat ascii 1.0\nelement vertex 1\n");
    assert!(matches!(validate(&missing_end), Err(E::Invalid(_))));
    let unknown_element = files.write(
        "edge.ply",
        b"ply\nformat ascii 1.0\nelement vertex 1\nproperty float x\nproperty float y\n\
          property float z\nelement edge 1\nend_header\n",
    );
    assert!(matches!(validate(&unknown_element), Err(E::Invalid(_))));
    let face_first = files.write(
        "order.ply",
        b"ply\nformat ascii 1.0\nelement face 1\nproperty list uchar int vertex_indices\n\
          element vertex 4\nproperty float x\nproperty float y\nproperty float z\nend_header\n",
    );
    assert!(matches!(validate(&face_first), Err(E::Invalid(_))));
    let duplicate_vertex = files.write(
        "twice.ply",
        b"ply\nformat ascii 1.0\nelement vertex 1\nelement vertex 2\nend_header\n",
    );
    assert!(matches!(validate(&duplicate_vertex), Err(E::Invalid(_))));
    let float_list = ASCII_TETRA.replace("list uchar int", "list float int");
    let path = files.write("float-list.ply", float_list.as_bytes());
    assert!(matches!(validate(&path), Err(E::Invalid(_))));
    let no_mesh = files.write(
        "empty.ply",
        b"ply\nformat ascii 1.0\nelement vertex 0\nproperty float x\nend_header\n",
    );
    assert!(matches!(validate(&no_mesh), Err(E::Invalid(_))));
    let face_without_vertex = files.write(
        "faceless.ply",
        b"ply\nformat ascii 1.0\nelement face 1\nproperty list uchar int vertex_indices\n\
          end_header\n",
    );
    assert!(matches!(validate(&face_without_vertex), Err(E::Invalid(_))));
}

#[test]
fn truncated_bodies_and_nonfinite_values_are_invalid() {
    let files = directory();
    let truncated = ASCII_TETRA.replace("0 0 1\n3 0 1 2", "0 0 1");
    let path = files.write("cut.ply", truncated.as_bytes());
    assert!(matches!(validate(&path), Err(E::Invalid(_))));
    let nan = ASCII_TETRA.replace("0 1 0\n", "nan 1 0\n");
    let path = files.write("nan.ply", nan.as_bytes());
    assert!(matches!(validate(&path), Err(E::Invalid(_))));
    let inf = ASCII_TETRA.replace("1 0 0\n", "inf 0 0\n");
    let path = files.write("inf.ply", inf.as_bytes());
    assert!(matches!(validate(&path), Err(E::Invalid(_))));
    let mut binary = binary_tetra(true);
    binary.truncate(binary.len() - 4);
    let path = files.write("cut.ply", &binary);
    assert!(matches!(validate(&path), Err(E::Invalid(_))));
    let mut binary = binary_tetra(true);
    // The last vertex's z value sits behind three full vertices and eight
    // bytes of the fourth; overwriting it with NaN must fail validation.
    let header_end = binary
        .windows(11)
        .position(|window| window == b"end_header\n")
        .unwrap()
        + 11;
    binary[header_end + 44..header_end + 48].copy_from_slice(&f32::NAN.to_le_bytes());
    let path = files.write("nan.ply", &binary);
    assert!(matches!(validate(&path), Err(E::Invalid(_))));
}

#[test]
fn face_values_are_range_checked_in_both_formats() {
    let files = directory();
    let out_of_range = ASCII_TETRA.replace("3 2 0 3", "3 2 0 4");
    let path = files.write("oob.ply", out_of_range.as_bytes());
    assert!(matches!(validate(&path), Err(E::Invalid(_))));
    let negative = ASCII_TETRA.replace("3 0 1 2", "3 0 -1 2");
    let path = files.write("neg.ply", negative.as_bytes());
    assert!(matches!(validate(&path), Err(E::Invalid(_))));
    let fractional_count = ASCII_TETRA.replace("3 0 1 2", "3.5 0 1 2");
    let path = files.write("frac.ply", fractional_count.as_bytes());
    assert!(matches!(validate(&path), Err(E::Invalid(_))));
    let fractional_index = ASCII_TETRA.replace("3 0 1 2", "3 0 1.5 2");
    let path = files.write("frac-index.ply", fractional_index.as_bytes());
    assert!(matches!(validate(&path), Err(E::Invalid(_))));
    let huge_list = ASCII_TETRA.replace("3 0 1 2", "1000001 0 1");
    let path = files.write("list.ply", huge_list.as_bytes());
    assert!(matches!(validate(&path), Err(E::Invalid(_))));
    let int_property = ASCII_TETRA
        .replace("property float z\n", "property int z\n")
        .replace("0 0 1\n", "0 0 0.5\n");
    let path = files.write("frac-int.ply", int_property.as_bytes());
    // z is declared int: a nonintegral scalar for it is rejected.
    match validate(&path) {
        Err(E::Invalid(error)) => assert!(error.to_string().contains("nonintegral")),
        other => panic!("expected nonintegral integer rejection, got {other:?}"),
    }
}

#[test]
fn injected_budgets_reject_without_reading_whole_files() {
    let files = directory();
    let path = files.write("tetra.ply", ASCII_TETRA.as_bytes());
    // The default budget passes the whole tetra, primes for restriction.
    validate_ply_file(&path, PlyValidationLimits::default(), || false).unwrap();
    let scan = PlyValidationLimits {
        vertices: 4,
        faces: 4,
        indices: 12,
        triangles: 10,
        ..Default::default()
    };
    validate_ply_file(&path, scan, || false).unwrap();
    // Twelve indices cross the seven-index budget during the third face.
    let loses_indices = PlyValidationLimits { indices: 7, ..scan };
    match validate_ply_file(&path, loses_indices, || false) {
        Err(E::Limit("face indices")) => {}
        other => panic!("expected index budget rejection, got {other:?}"),
    }
    // The fourth face's fan pushes the triangle count past three.
    let loses_triangles = PlyValidationLimits {
        triangles: 3,
        ..scan
    };
    match validate_ply_file(&path, loses_triangles, || false) {
        Err(E::Limit("triangles")) => {}
        other => panic!("expected triangle budget rejection, got {other:?}"),
    }
    // A single 200-digit ASCII token hits the scalar token guard.
    let long = ASCII_TETRA.replace("3 0 1 2\n", &format!("3 0 1 {}\n", "2".repeat(200)));
    let path = files.write("long.ply", long.as_bytes());
    match validate(&path) {
        Err(E::Limit("scalar token bytes")) => {}
        other => panic!("expected token guard, got {other:?}"),
    }
    // Ten million declared vertices with a three-vertex body: the scan
    // reaches EOF after the first vertex line and fails without ever
    // trusting the declared count as an allocation.
    let lie = ASCII_TETRA.replace("element vertex 4\n", "element vertex 10000000\n");
    let path = files.write("liar.ply", lie.as_bytes());
    assert!(matches!(validate(&path), Err(E::Invalid(_))));
}

#[test]
fn cancellation_is_honoured_before_and_during_the_scan() {
    let files = directory();
    let path = files.write("tetra.ply", ASCII_TETRA.as_bytes());
    match validate_ply_file(&path, PlyValidationLimits::default(), || true) {
        Err(Cancelled) => {}
        other => panic!("expected cancellation, got {other:?}"),
    }
    let calls = std::cell::Cell::new(0);
    let result = validate_ply_file(&path, PlyValidationLimits::default(), || {
        calls.set(calls.get() + 1);
        calls.get() > 1
    });
    assert!(matches!(result, Err(Cancelled)), "{result:?}");
    // The scan checks cancellation at each vertex and face; with nine
    // checks in this fixture, a threshold of five fires mid-face-scan.
    let calls = std::cell::Cell::new(0);
    let result = validate_ply_file(&path, PlyValidationLimits::default(), || {
        calls.set(calls.get() + 1);
        calls.get() > 5
    });
    match result {
        Err(Cancelled) => {}
        Ok(info) => panic!("cancellation mid-scan validated a whole file: {info:?}"),
        other => panic!("expected cancellation, got {other:?}"),
    }
}

#[test]
fn ascii_validation_rejects_what_the_rendering_reader_rejects() {
    let files = directory();
    // The renderer parses face indices as u32: 2^32 is out of its range
    // even though it parses as a finite f64.
    let overflowing = ASCII_TETRA.replace("3 0 1 2", "3 0 1 4294967296");
    let path = files.write("u32.ply", overflowing.as_bytes());
    assert!(matches!(validate(&path), Err(E::Invalid(_))));
    // The reader parses face counts as usize and indices as u32; integral
    // floating notation parses here as a zero-fraction f64, so validation
    // must reject what the later mesh read would reject.
    let count_notation = ASCII_TETRA.replace("3 0 1 2", "3.0 0 1 2");
    let path = files.write("count-notation.ply", count_notation.as_bytes());
    assert!(
        matches!(validate(&path), Err(E::Invalid(_))),
        "3.0 as a face count must fail before publication"
    );
    let index_notation = ASCII_TETRA.replace("3 0 1 2", "3 0 1.0 2");
    let path = files.write("index-notation.ply", index_notation.as_bytes());
    assert!(
        matches!(validate(&path), Err(E::Invalid(_))),
        "1.0 as a face index must fail before publication"
    );
    // "+3" is the integer syntax usize::from_str accepts.
    let plus = ASCII_TETRA.replace("3 0 1 2", "+3 0 1 2");
    let path = files.write("plus.ply", plus.as_bytes());
    validate(&path).unwrap();
    // end_header with CRLF still terminates the header.
    let crlf = ASCII_TETRA.replace('\n', "\r\n");
    let path = files.write("crlf.ply", crlf.as_bytes());
    validate(&path).unwrap();
}
