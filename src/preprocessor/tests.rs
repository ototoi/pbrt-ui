//! Tests for the preprocessor

use super::*;
use std::fs;
use tempfile::TempDir;

#[test]
fn test_simple_define() {
    let mut preprocessor = Preprocessor::new();
    let source = r#"
#define PI 3.14159
let radius = 10.0;
let circumference = 2.0 * PI * radius;
"#;
    
    let result = preprocessor.process(source).unwrap();
    assert!(result.contains("2.0 * 3.14159 * radius"));
}

#[test]
fn test_multiple_defines() {
    let mut preprocessor = Preprocessor::new();
    let source = r#"
#define WIDTH 800
#define HEIGHT 600
let area = WIDTH * HEIGHT;
"#;
    
    let result = preprocessor.process(source).unwrap();
    assert!(result.contains("800 * 600"));
}

#[test]
fn test_macro_with_params() {
    let mut preprocessor = Preprocessor::new();
    let source = r#"
#define MAX(a, b) ((a) > (b) ? (a) : (b))
let maximum = MAX(x, y);
"#;
    
    let result = preprocessor.process(source).unwrap();
    assert!(result.contains("((x) > (y) ? (x) : (y))"));
}

#[test]
fn test_macro_with_multiple_params() {
    let mut preprocessor = Preprocessor::new();
    let source = r#"
#define CLAMP(val, min, max) ((val) < (min) ? (min) : ((val) > (max) ? (max) : (val)))
let clamped = CLAMP(value, 0.0, 1.0);
"#;
    
    let result = preprocessor.process(source).unwrap();
    assert!(result.contains("((value) < (0.0) ? (0.0) : ((value) > (1.0) ? (1.0) : (value)))"));
}

#[test]
fn test_ifdef_defined() {
    let mut preprocessor = Preprocessor::new();
    preprocessor.define("DEBUG", "1");
    
    let source = r#"
#ifdef DEBUG
let debug_mode = true;
#endif
let normal_code = 1;
"#;
    
    let result = preprocessor.process(source).unwrap();
    assert!(result.contains("debug_mode"));
    assert!(result.contains("normal_code"));
}

#[test]
fn test_ifdef_not_defined() {
    let mut preprocessor = Preprocessor::new();
    
    let source = r#"
#ifdef DEBUG
let debug_mode = true;
#endif
let normal_code = 1;
"#;
    
    let result = preprocessor.process(source).unwrap();
    assert!(!result.contains("debug_mode"));
    assert!(result.contains("normal_code"));
}

#[test]
fn test_ifndef_not_defined() {
    let mut preprocessor = Preprocessor::new();
    
    let source = r#"
#ifndef RELEASE
let debug_mode = true;
#endif
let normal_code = 1;
"#;
    
    let result = preprocessor.process(source).unwrap();
    assert!(result.contains("debug_mode"));
    assert!(result.contains("normal_code"));
}

#[test]
fn test_ifndef_defined() {
    let mut preprocessor = Preprocessor::new();
    preprocessor.define("RELEASE", "1");
    
    let source = r#"
#ifndef RELEASE
let debug_mode = true;
#endif
let normal_code = 1;
"#;
    
    let result = preprocessor.process(source).unwrap();
    assert!(!result.contains("debug_mode"));
    assert!(result.contains("normal_code"));
}

#[test]
fn test_nested_conditionals() {
    let mut preprocessor = Preprocessor::new();
    preprocessor.define("FEATURE_A", "1");
    preprocessor.define("FEATURE_B", "1");
    
    let source = r#"
#ifdef FEATURE_A
let feature_a = true;
#ifdef FEATURE_B
let feature_b = true;
#endif
#endif
let normal_code = 1;
"#;
    
    let result = preprocessor.process(source).unwrap();
    assert!(result.contains("feature_a"));
    assert!(result.contains("feature_b"));
    assert!(result.contains("normal_code"));
}

#[test]
fn test_nested_conditionals_partial() {
    let mut preprocessor = Preprocessor::new();
    preprocessor.define("FEATURE_A", "1");
    // FEATURE_B is not defined
    
    let source = r#"
#ifdef FEATURE_A
let feature_a = true;
#ifdef FEATURE_B
let feature_b = true;
#endif
let after_b = true;
#endif
let normal_code = 1;
"#;
    
    let result = preprocessor.process(source).unwrap();
    assert!(result.contains("feature_a"));
    assert!(!result.contains("feature_b"));
    assert!(result.contains("after_b"));
    assert!(result.contains("normal_code"));
}

#[test]
fn test_include_simple() {
    let temp_dir = TempDir::new().unwrap();
    let include_path = temp_dir.path().join("common.wgsl");
    fs::write(&include_path, "let included_value = 42;").unwrap();
    
    let mut preprocessor = Preprocessor::with_base_path(temp_dir.path());
    let source = r#"
#include "common.wgsl"
let main_value = 100;
"#;
    
    let result = preprocessor.process(source).unwrap();
    assert!(result.contains("included_value"));
    assert!(result.contains("main_value"));
}

#[test]
fn test_include_with_defines() {
    let temp_dir = TempDir::new().unwrap();
    let include_path = temp_dir.path().join("config.wgsl");
    fs::write(&include_path, "#define SIZE 256").unwrap();
    
    let mut preprocessor = Preprocessor::with_base_path(temp_dir.path());
    let source = r#"
#include "config.wgsl"
let array_size = SIZE;
"#;
    
    let result = preprocessor.process(source).unwrap();
    assert!(result.contains("array_size = 256"));
}

#[test]
fn test_circular_dependency() {
    let temp_dir = TempDir::new().unwrap();
    
    // Create file A that includes file B
    let file_a = temp_dir.path().join("a.wgsl");
    fs::write(&file_a, r#"#include "b.wgsl""#).unwrap();
    
    // Create file B that includes file A
    let file_b = temp_dir.path().join("b.wgsl");
    fs::write(&file_b, r#"#include "a.wgsl""#).unwrap();
    
    let mut preprocessor = Preprocessor::with_base_path(temp_dir.path());
    let source = r#"#include "a.wgsl""#;
    
    let result = preprocessor.process(source);
    assert!(result.is_err());
    match result {
        Err(PreprocessorError::CircularDependency { .. }) => {},
        _ => panic!("Expected CircularDependency error"),
    }
}

#[test]
fn test_file_not_found() {
    let temp_dir = TempDir::new().unwrap();
    let mut preprocessor = Preprocessor::with_base_path(temp_dir.path());
    let source = r#"#include "nonexistent.wgsl""#;
    
    let result = preprocessor.process(source);
    assert!(result.is_err());
    match result {
        Err(PreprocessorError::IoError { .. }) => {},
        _ => panic!("Expected IoError"),
    }
}

#[test]
fn test_unclosed_ifdef() {
    let mut preprocessor = Preprocessor::new();
    let source = r#"
#ifdef DEBUG
let debug_mode = true;
"#;
    
    let result = preprocessor.process(source);
    assert!(result.is_err());
    match result {
        Err(PreprocessorError::ParseError { message, .. }) => {
            assert!(message.contains("unclosed"));
        },
        _ => panic!("Expected ParseError for unclosed conditional"),
    }
}

#[test]
fn test_unexpected_endif() {
    let mut preprocessor = Preprocessor::new();
    let source = r#"
let value = 1;
#endif
"#;
    
    let result = preprocessor.process(source);
    assert!(result.is_err());
    match result {
        Err(PreprocessorError::ParseError { message, .. }) => {
            assert!(message.contains("Unexpected #endif"));
        },
        _ => panic!("Expected ParseError for unexpected endif"),
    }
}

#[test]
fn test_macro_wrong_arg_count() {
    let mut preprocessor = Preprocessor::new();
    let source = r#"
#define ADD(a, b) ((a) + (b))
let result = ADD(1);
"#;
    
    let result = preprocessor.process(source);
    assert!(result.is_err());
    match result {
        Err(PreprocessorError::InvalidMacro { .. }) => {},
        _ => panic!("Expected InvalidMacro error"),
    }
}

#[test]
fn test_define_empty_value() {
    let mut preprocessor = Preprocessor::new();
    let source = r#"
#define EMPTY
let value = 1;
"#;
    
    let result = preprocessor.process(source).unwrap();
    assert!(result.contains("let value = 1"));
}

#[test]
fn test_predefined_symbol() {
    let mut preprocessor = Preprocessor::new();
    preprocessor.define("VERSION", "100");
    
    let source = r#"
let version = VERSION;
"#;
    
    let result = preprocessor.process(source).unwrap();
    assert!(result.contains("version = 100"));
}

#[test]
fn test_complex_example() {
    let temp_dir = TempDir::new().unwrap();
    
    // Create a common header file
    let common_path = temp_dir.path().join("common.wgsl");
    fs::write(&common_path, r#"
#define PI 3.14159
#define TWO_PI (2.0 * PI)
"#).unwrap();
    
    let mut preprocessor = Preprocessor::with_base_path(temp_dir.path());
    preprocessor.define("USE_OPTIMIZATIONS", "1");
    
    let source = r#"
#include "common.wgsl"
#define SQUARE(x) ((x) * (x))

#ifdef USE_OPTIMIZATIONS
let optimized = true;
#endif

let circle_area = SQUARE(radius) * PI;
let full_rotation = TWO_PI;
"#;
    
    let result = preprocessor.process(source).unwrap();
    assert!(result.contains("optimized = true"));
    assert!(result.contains("((radius) * (radius)) * 3.14159"));
    assert!(result.contains("(2.0 * 3.14159)"));
}

#[test]
fn test_macro_nested_calls() {
    let mut preprocessor = Preprocessor::new();
    let source = r#"
#define DOUBLE(x) ((x) * 2)
#define QUAD(x) DOUBLE(DOUBLE(x))
let result = QUAD(5);
"#;
    
    let result = preprocessor.process(source).unwrap();
    // After first expansion: DOUBLE(DOUBLE(5))
    // After second pass: DOUBLE(((5) * 2))
    // After third pass: ((((5) * 2)) * 2)
    assert!(result.contains("((((5) * 2)) * 2)"));
}

#[test]
fn test_preserve_non_directive_lines() {
    let mut preprocessor = Preprocessor::new();
    let source = r#"
// This is a comment
fn main() {
    let x = 1;
    #define VALUE 42
    let y = VALUE;
}
"#;
    
    let result = preprocessor.process(source).unwrap();
    assert!(result.contains("// This is a comment"));
    assert!(result.contains("fn main()"));
    assert!(result.contains("let y = 42"));
}

#[test]
fn test_macro_no_args() {
    let mut preprocessor = Preprocessor::new();
    let source = r#"
#define RESET() 0
let value = RESET();
"#;
    
    let result = preprocessor.process(source).unwrap();
    assert!(result.contains("let value = 0"));
}

#[test]
fn test_identifier_starting_with_number() {
    let mut preprocessor = Preprocessor::new();
    // This should NOT be parsed as a valid identifier define
    let source = r#"
#define 1ABC invalid
let value = 1;
"#;
    
    // Should fail to parse the directive
    let result = preprocessor.process(source);
    assert!(result.is_err());
}
