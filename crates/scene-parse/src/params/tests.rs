use super::*;

fn parse(source: &str) -> Vec<Directive> {
    let mut lexer = Lexer::new("test.pbrt", source);
    let mut reader = DirectiveReader::new(&mut lexer);
    let mut out = Vec::new();
    while let Some(directive) = reader.read_directive().expect("parse") {
        out.push(directive);
    }
    out
}

#[test]
fn shape_keeps_its_type_as_positional_not_a_parameter() {
    let directives = parse("Shape \"plymesh\"  \"string filename\" [ \"geometry/mesh.ply\" ]\n");
    assert_eq!(directives.len(), 1);
    let directive = &directives[0];
    assert_eq!(directive.name, "Shape");
    assert_eq!(directive.positional_text(0), Some("plymesh"));
    assert_eq!(directive.parameters.len(), 1);
    assert_eq!(directive.parameters[0].name, "filename");
    assert_eq!(
        directive.parameters[0].values[0].as_text(),
        Some("geometry/mesh.ply")
    );
}

#[test]
fn texture_takes_three_quoted_positionals() {
    let directives =
        parse("Texture \"float\" \"Kd\" \"scale\"  \"point2 P1\" [ 0 0 ]  \"point2 P2\" [ 1 1 ]");
    let directive = &directives[0];
    assert_eq!(directive.positional.len(), 3);
    assert_eq!(directive.positional_text(2), Some("scale"));
    assert_eq!(directive.parameters.len(), 2);
}

#[test]
fn parameter_type_declaration_ignores_leading_whitespace() {
    let directives = parse("Material \"hair\" \"float beta_m\" 0.5 \" float beta_n\" 0.25");
    let params = directives[0].params();
    assert_eq!(params.float("beta_m", 0.0), 0.5);
    assert_eq!(params.float("beta_n", 0.0), 0.25);
}

#[test]
fn transform_reads_sixteen_positional_numbers() {
    let directives = parse("ConcatTransform 1 0 0 0  0 1 0 0  0 0 1 0  0 0 0 1\nScale 2 2 2");
    assert_eq!(directives[0].positional.len(), 16);
    assert_eq!(directives[1].name, "Scale");
    assert_eq!(directives[1].positional.len(), 3);
}

#[test]
fn transform_reads_bracketed_matrix_and_continues_with_next_directive() {
    let directives = parse("Transform [ 1 0 0 0  0 1 0 0  0 0 1 0  0 0 0 1 ]\nIdentity");
    assert_eq!(directives[0].name, "Transform");
    assert_eq!(directives[0].positional.len(), 16);
    assert_eq!(directives[1].name, "Identity");
}

#[test]
fn object_begin_name_is_optional() {
    let directives = parse("ObjectBegin\nObjectEnd\nObjectBegin \"part\"\nObjectEnd");
    assert_eq!(directives[0].positional.len(), 0);
    assert_eq!(directives[2].positional_text(0), Some("part"));
    assert_eq!(directives[1].name, "ObjectEnd");
    assert_eq!(directives[3].name, "ObjectEnd");
}

#[test]
fn option_records_name_and_value() {
    let directives = parse("Option \"bool\" true\nOption \"string\" [ 64 64 ]");
    assert_eq!(directives[0].positional.len(), 2);
    assert_eq!(directives[0].positional_text(0), Some("bool"));
    assert_eq!(directives[0].positional[1], Value::Text("true".into()));
    assert_eq!(directives[1].positional_text(0), Some("string"));
    assert_eq!(directives[1].positional.len(), 3);
}

#[test]
fn active_transform_takes_a_word() {
    let directives = parse("ActiveTransform Start\nActiveTransform All");
    assert_eq!(directives[0].positional_text(0), Some("Start"));
    assert_eq!(directives[1].positional_text(0), Some("All"));
}

#[test]
fn single_value_parameter_without_brackets_is_accepted() {
    let directives = parse("Camera \"perspective\"  \"float fov\" 47");
    let params = directives[0].params();
    assert_eq!(params.float("fov", 0.0), 47.0);
}

#[test]
fn points_group_flat_numbers_into_triples() {
    let directives = parse(
        "Shape \"trianglemesh\" \"point P\" [ 0 0 0 1 0 0 0 1 0 ] \"integer indices\" [ 0 1 2 ]",
    );
    let params = directives[0].params();
    assert_eq!(
        params.points("P"),
        vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]
    );
    assert_eq!(params.ints("indices"), vec![0, 1, 2]);
}

#[test]
fn inf_literal_flows_through_parameters() {
    let directives = parse("Shape \"cylinder\" \"float zmin\" [ -inf ] \"float zmax\" [ inf ]");
    let params = directives[0].params();
    assert_eq!(params.float("zmin", 0.0), f64::NEG_INFINITY);
    assert_eq!(params.float("zmax", 0.0), f64::INFINITY);
}

#[test]
fn missing_positional_is_a_syntax_error() {
    let error = {
        let mut lexer = Lexer::new("bad.pbrt", "Translate 1 2\n");
        let mut reader = DirectiveReader::new(&mut lexer);
        reader.read_directive().expect_err("should fail")
    };
    match error {
        SceneParseError::Syntax { path, line, .. } => {
            assert_eq!(path, "bad.pbrt");
            assert_eq!(line, 1);
        }
        other => panic!("unexpected error: {other}"),
    }
}

/// Classification runs the real lexer; results must match what the text
/// reader produces for the same lexemes.
#[test]
fn classify_matches_the_reader_for_special_numbers_and_bools() {
    // inf / -inf / nan are Words to the lexer, hence Text (with the
    // numeric escape living in Value::as_float).
    assert_eq!(Value::classify("inf"), Value::Text("inf".into()));
    assert_eq!(Value::classify("-inf"), Value::Text("-inf".into()));
    assert_eq!(Value::classify("nan"), Value::Text("nan".into()));
    assert_eq!(Value::classify("inf").as_float(), Some(f64::INFINITY));
    assert_eq!(Value::classify("-inf").as_float(), Some(f64::NEG_INFINITY));
    assert!(Value::classify("nan").as_float().unwrap().is_nan());
    // Booleans are unquoted words.
    assert_eq!(Value::classify("true"), Value::Text("true".into()));
    assert_eq!(Value::classify("false"), Value::Text("false".into()));
    // Both spellings classify as numbers.
    assert_eq!(Value::classify("1"), Value::Number(1.0));
    assert_eq!(Value::classify("1.0"), Value::Number(1.0));
    assert_eq!(Value::classify("-2.5e-3"), Value::Number(-2.5e-3));
}

/// Quoted escapes decode the same way in classification and reading.
#[test]
fn classify_decodes_quoted_escapes_like_the_reader() {
    assert_eq!(Value::classify("\"a b\""), Value::Text("a b".into()));
    assert_eq!(Value::classify("\"q\\\"w\""), Value::Text("q\"w".into()));
    assert_eq!(Value::classify("\"q\\\\w\""), Value::Text("q\\w".into()));
    assert_eq!(Value::classify("\"q\\nw\""), Value::Text("q\nw".into()));
    let reader = parse("Shape \"sphere\" \"string name\" [ \"n\" ]");
    assert_eq!(Value::classify("\"n\""), reader[0].parameters[0].values[0]);
}

/// Values that are not a single token stay text; classify never panics.
#[test]
fn classify_rejects_multi_token_lexemes() {
    assert_eq!(Value::classify("1 2"), Value::Text("1 2".into()));
    assert_eq!(Value::classify(""), Value::Text(String::new()));
    assert_eq!(Value::classify("[1]"), Value::Text("[1]".into()));
}
