use scene_parse::{lexer::Lexer, params::DirectiveReader};

#[test]
fn byte_ranges_and_numeric_lexemes_survive_lookahead_on_the_same_line() {
    let source = "# 前置き\nShape \"sphere\" \"integer custom\" [9007199254740993] Translate 1e+2 0 0 # end\n";
    let mut lexer = Lexer::new("test.pbrt", source);
    let mut reader = DirectiveReader::new(&mut lexer);
    let first = reader.read_directive().unwrap().unwrap();
    let second = reader.read_directive().unwrap().unwrap();
    assert_eq!(
        &source[first.offset as usize..(first.offset + first.length) as usize],
        "Shape \"sphere\" \"integer custom\" [9007199254740993]"
    );
    assert_eq!(first.parameters[0].value_tokens, ["9007199254740993"]);
    assert_eq!(second.positional_tokens, ["1e+2", "0", "0"]);
    assert_eq!(
        &source[second.offset as usize..(second.offset + second.length) as usize],
        "Translate 1e+2 0 0"
    );
}

#[test]
fn native_reader_can_bound_tokens_and_rejects_nested_value_lists() {
    let mut lexer = Lexer::new("test.pbrt", "Shape \"sphere\" \"float radius\" [1 2 3]");
    assert!(
        DirectiveReader::with_token_limit(&mut lexer, 5)
            .read_directive()
            .is_err()
    );
    let mut lexer = Lexer::new("test.pbrt", "Shape \"sphere\" \"float radius\" [[1]]");
    assert!(DirectiveReader::new(&mut lexer).read_directive().is_err());
}
