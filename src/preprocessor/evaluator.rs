//! Expression evaluator for #if and #elif directives
//!
//! This module provides parsing and evaluation of C-like boolean expressions
//! used in preprocessor conditionals.

use super::error::{PreprocessorError, PreprocessorResult};
use nom::{
    IResult,
    branch::alt,
    bytes::complete::tag,
    character::complete::{alpha1, alphanumeric1, char, digit1, space0},
    combinator::{map, recognize},
    multi::many0,
    sequence::{delimited, preceded, tuple},
};

/// Represents a parsed expression
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// Integer literal
    Integer(i64),
    /// Identifier (symbol/define name)
    Identifier(String),
    /// Defined operator (checks if symbol exists)
    Defined(String),
    /// Logical NOT
    Not(Box<Expr>),
    /// Logical AND
    And(Box<Expr>, Box<Expr>),
    /// Logical OR
    Or(Box<Expr>, Box<Expr>),
    /// Equality comparison
    Equal(Box<Expr>, Box<Expr>),
    /// Inequality comparison
    NotEqual(Box<Expr>, Box<Expr>),
    /// Less than
    LessThan(Box<Expr>, Box<Expr>),
    /// Greater than
    GreaterThan(Box<Expr>, Box<Expr>),
    /// Less than or equal
    LessEqual(Box<Expr>, Box<Expr>),
    /// Greater than or equal
    GreaterEqual(Box<Expr>, Box<Expr>),
}

/// Parse an integer literal
fn parse_integer(input: &str) -> IResult<&str, Expr> {
    map(preceded(space0, digit1), |s: &str| {
        Expr::Integer(
            s.parse()
                .expect("nom's digit1 parser guarantees valid decimal digits"),
        )
    })(input)
}

/// Parse an identifier
fn parse_identifier(input: &str) -> IResult<&str, Expr> {
    let (input, _) = space0(input)?;
    map(
        recognize(tuple((
            alt((tag("_"), recognize(alpha1))),
            many0(alt((alphanumeric1, tag("_")))),
        ))),
        |s: &str| Expr::Identifier(s.to_string()),
    )(input)
}

/// Parse the defined operator: defined(SYMBOL) or defined SYMBOL
fn parse_defined(input: &str) -> IResult<&str, Expr> {
    let (input, _) = space0(input)?;
    let (input, _) = tag("defined")(input)?;
    let (input, _) = space0(input)?;

    // Try to parse with parentheses first: defined(SYMBOL)
    if let Ok((input, _)) = char::<_, nom::error::Error<_>>('(')(input) {
        let (input, _) = space0(input)?;
        let (input, symbol) = recognize(tuple((
            alt((tag("_"), recognize(alpha1))),
            many0(alt((alphanumeric1, tag("_")))),
        )))(input)?;
        let (input, _) = space0(input)?;
        let (input, _) = char(')')(input)?;
        Ok((input, Expr::Defined(symbol.to_string())))
    } else {
        // Parse without parentheses: defined SYMBOL
        let (input, symbol) = recognize(tuple((
            alt((tag("_"), recognize(alpha1))),
            many0(alt((alphanumeric1, tag("_")))),
        )))(input)?;
        Ok((input, Expr::Defined(symbol.to_string())))
    }
}

/// Parse a primary expression (integer, identifier, or parenthesized expression)
fn parse_primary(input: &str) -> IResult<&str, Expr> {
    let (input, _) = space0(input)?;
    alt((
        parse_integer,
        parse_defined,
        parse_identifier,
        delimited(char('('), parse_or_expr, preceded(space0, char(')'))),
    ))(input)
}

/// Parse a unary expression (!, or primary)
fn parse_unary(input: &str) -> IResult<&str, Expr> {
    let (input, _) = space0(input)?;
    alt((
        map(preceded(char('!'), parse_unary), |expr| {
            Expr::Not(Box::new(expr))
        }),
        parse_primary,
    ))(input)
}

/// Parse comparison operators
fn parse_comparison(input: &str) -> IResult<&str, Expr> {
    let (input, left) = parse_unary(input)?;
    let (input, _) = space0(input)?;

    // Try to parse a comparison operator and create the appropriate expression
    // Parse the operator first, then construct the expression once
    let op_result: IResult<&str, i32> = alt((
        map(tag("=="), |_| 0),
        map(tag("!="), |_| 1),
        map(tag("<="), |_| 2),
        map(tag(">="), |_| 3),
        map(char('<'), |_| 4),
        map(char('>'), |_| 5),
    ))(input);

    match op_result {
        Ok((input, op)) => {
            let (input, right) = parse_unary(input)?;
            let expr = match op {
                0 => Expr::Equal(Box::new(left), Box::new(right)),
                1 => Expr::NotEqual(Box::new(left), Box::new(right)),
                2 => Expr::LessEqual(Box::new(left), Box::new(right)),
                3 => Expr::GreaterEqual(Box::new(left), Box::new(right)),
                4 => Expr::LessThan(Box::new(left), Box::new(right)),
                5 => Expr::GreaterThan(Box::new(left), Box::new(right)),
                _ => unreachable!("operator index must be 0-5 based on alt() above"),
            };
            Ok((input, expr))
        }
        Err(_) => Ok((input, left)),
    }
}

/// Parse AND expressions (left-associative, iterative)
fn parse_and_expr(input: &str) -> IResult<&str, Expr> {
    let (mut input, mut left) = parse_comparison(input)?;

    loop {
        let (i, _) = space0(input)?;
        if let Ok((i, _)) = tag::<_, _, nom::error::Error<_>>("&&")(i) {
            let (i, right) = parse_comparison(i)?;
            left = Expr::And(Box::new(left), Box::new(right));
            input = i;
        } else {
            return Ok((input, left));
        }
    }
}

/// Parse OR expressions (left-associative, iterative)
fn parse_or_expr(input: &str) -> IResult<&str, Expr> {
    let (mut input, mut left) = parse_and_expr(input)?;

    loop {
        let (i, _) = space0(input)?;
        if let Ok((i, _)) = tag::<_, _, nom::error::Error<_>>("||")(i) {
            let (i, right) = parse_and_expr(i)?;
            left = Expr::Or(Box::new(left), Box::new(right));
            input = i;
        } else {
            return Ok((input, left));
        }
    }
}

/// Parse a complete expression
pub fn parse_expr(input: &str) -> IResult<&str, Expr> {
    let (input, expr) = parse_or_expr(input)?;
    let (input, _) = space0(input)?;
    Ok((input, expr))
}

/// Evaluate an expression to a boolean value
///
/// # Arguments
/// * `expr` - The expression to evaluate
/// * `defines` - HashMap of defined symbols with their values (for value lookups)
/// * `all_defined_names` - HashSet of all defined names (includes both defines and macros, for `defined()` checks)
pub fn evaluate(
    expr: &Expr,
    defines: &std::collections::HashMap<String, String>,
    all_defined_names: &std::collections::HashSet<String>,
) -> PreprocessorResult<bool> {
    match expr {
        Expr::Integer(val) => Ok(*val != 0),
        Expr::Identifier(name) => {
            // Check if the identifier is defined
            if let Some(value) = defines.get(name) {
                // Try to parse as integer
                if let Ok(num) = value.parse::<i64>() {
                    Ok(num != 0)
                } else {
                    // Non-numeric value is treated as true if defined
                    Ok(true)
                }
            } else {
                // Undefined symbols are treated as false (0)
                Ok(false)
            }
        }
        Expr::Defined(name) => {
            // Return true if the symbol is present in either defines or macros
            Ok(all_defined_names.contains(name))
        }
        Expr::Not(inner) => {
            let val = evaluate(inner, defines, all_defined_names)?;
            Ok(!val)
        }
        Expr::And(left, right) => {
            let left_val = evaluate(left, defines, all_defined_names)?;
            let right_val = evaluate(right, defines, all_defined_names)?;
            Ok(left_val && right_val)
        }
        Expr::Or(left, right) => {
            let left_val = evaluate(left, defines, all_defined_names)?;
            let right_val = evaluate(right, defines, all_defined_names)?;
            Ok(left_val || right_val)
        }
        Expr::Equal(left, right) => {
            let left_num = evaluate_to_number(left, defines, all_defined_names)?;
            let right_num = evaluate_to_number(right, defines, all_defined_names)?;
            Ok(left_num == right_num)
        }
        Expr::NotEqual(left, right) => {
            let left_num = evaluate_to_number(left, defines, all_defined_names)?;
            let right_num = evaluate_to_number(right, defines, all_defined_names)?;
            Ok(left_num != right_num)
        }
        Expr::LessThan(left, right) => {
            let left_num = evaluate_to_number(left, defines, all_defined_names)?;
            let right_num = evaluate_to_number(right, defines, all_defined_names)?;
            Ok(left_num < right_num)
        }
        Expr::GreaterThan(left, right) => {
            let left_num = evaluate_to_number(left, defines, all_defined_names)?;
            let right_num = evaluate_to_number(right, defines, all_defined_names)?;
            Ok(left_num > right_num)
        }
        Expr::LessEqual(left, right) => {
            let left_num = evaluate_to_number(left, defines, all_defined_names)?;
            let right_num = evaluate_to_number(right, defines, all_defined_names)?;
            Ok(left_num <= right_num)
        }
        Expr::GreaterEqual(left, right) => {
            let left_num = evaluate_to_number(left, defines, all_defined_names)?;
            let right_num = evaluate_to_number(right, defines, all_defined_names)?;
            Ok(left_num >= right_num)
        }
    }
}

/// Helper to evaluate an expression to a numeric value
fn evaluate_to_number(
    expr: &Expr,
    defines: &std::collections::HashMap<String, String>,
    all_defined_names: &std::collections::HashSet<String>,
) -> PreprocessorResult<i64> {
    match expr {
        Expr::Integer(val) => Ok(*val),
        Expr::Identifier(name) => {
            if let Some(value) = defines.get(name) {
                value
                    .parse::<i64>()
                    .map_err(|_| PreprocessorError::ParseError {
                        line: 0,
                        message: format!("Cannot convert '{}' to number", value),
                    })
            } else {
                // Undefined symbols are treated as 0
                Ok(0)
            }
        }
        _ => {
            // For other expression types, evaluate to bool then convert to number
            let bool_val = evaluate(expr, defines, all_defined_names)?;
            Ok(if bool_val { 1 } else { 0 })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

    /// Helper function to create a HashSet of all defined names from a HashMap
    fn make_all_defined(defines: &HashMap<String, String>) -> HashSet<String> {
        defines.keys().cloned().collect()
    }

    #[test]
    fn test_parse_integer() {
        let result = parse_expr("42");
        assert!(result.is_ok());
        let (_, expr) = result.unwrap();
        assert_eq!(expr, Expr::Integer(42));
    }

    #[test]
    fn test_parse_identifier() {
        let result = parse_expr("DEBUG");
        assert!(result.is_ok());
        let (_, expr) = result.unwrap();
        assert_eq!(expr, Expr::Identifier("DEBUG".to_string()));
    }

    #[test]
    fn test_parse_not() {
        let result = parse_expr("!DEBUG");
        assert!(result.is_ok());
        let (_, expr) = result.unwrap();
        assert_eq!(
            expr,
            Expr::Not(Box::new(Expr::Identifier("DEBUG".to_string())))
        );
    }

    #[test]
    fn test_parse_and() {
        let result = parse_expr("A && B");
        assert!(result.is_ok());
        let (_, expr) = result.unwrap();
        assert_eq!(
            expr,
            Expr::And(
                Box::new(Expr::Identifier("A".to_string())),
                Box::new(Expr::Identifier("B".to_string()))
            )
        );
    }

    #[test]
    fn test_parse_or() {
        let result = parse_expr("A || B");
        assert!(result.is_ok());
        let (_, expr) = result.unwrap();
        assert_eq!(
            expr,
            Expr::Or(
                Box::new(Expr::Identifier("A".to_string())),
                Box::new(Expr::Identifier("B".to_string()))
            )
        );
    }

    #[test]
    fn test_parse_parentheses() {
        let result = parse_expr("(A && B) || C");
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_comparison_equal() {
        let result = parse_expr("X == 1");
        assert!(result.is_ok());
        let (_, expr) = result.unwrap();
        assert_eq!(
            expr,
            Expr::Equal(
                Box::new(Expr::Identifier("X".to_string())),
                Box::new(Expr::Integer(1))
            )
        );
    }

    #[test]
    fn test_parse_comparison_not_equal() {
        let result = parse_expr("X != 0");
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_less_than() {
        let result = parse_expr("X < 10");
        assert!(result.is_ok());
    }

    #[test]
    fn test_evaluate_integer_true() {
        let expr = Expr::Integer(1);
        let defines = HashMap::new();
        assert!(evaluate(&expr, &defines, &make_all_defined(&defines)).unwrap());
    }

    #[test]
    fn test_evaluate_integer_false() {
        let expr = Expr::Integer(0);
        let defines = HashMap::new();
        assert!(!evaluate(&expr, &defines, &make_all_defined(&defines)).unwrap());
    }

    #[test]
    fn test_evaluate_identifier_defined() {
        let expr = Expr::Identifier("DEBUG".to_string());
        let mut defines = HashMap::new();
        defines.insert("DEBUG".to_string(), "1".to_string());
        assert!(evaluate(&expr, &defines, &make_all_defined(&defines)).unwrap());
    }

    #[test]
    fn test_evaluate_identifier_undefined() {
        let expr = Expr::Identifier("DEBUG".to_string());
        let defines = HashMap::new();
        assert!(!evaluate(&expr, &defines, &make_all_defined(&defines)).unwrap());
    }

    #[test]
    fn test_evaluate_not() {
        let expr = Expr::Not(Box::new(Expr::Integer(0)));
        let defines = HashMap::new();
        assert!(evaluate(&expr, &defines, &make_all_defined(&defines)).unwrap());
    }

    #[test]
    fn test_evaluate_and() {
        let expr = Expr::And(Box::new(Expr::Integer(1)), Box::new(Expr::Integer(1)));
        let defines = HashMap::new();
        assert!(evaluate(&expr, &defines, &make_all_defined(&defines)).unwrap());
    }

    #[test]
    fn test_evaluate_or() {
        let expr = Expr::Or(Box::new(Expr::Integer(0)), Box::new(Expr::Integer(1)));
        let defines = HashMap::new();
        assert!(evaluate(&expr, &defines, &make_all_defined(&defines)).unwrap());
    }

    #[test]
    fn test_evaluate_comparison() {
        let expr = Expr::Equal(Box::new(Expr::Integer(5)), Box::new(Expr::Integer(5)));
        let defines = HashMap::new();
        assert!(evaluate(&expr, &defines, &make_all_defined(&defines)).unwrap());
    }

    #[test]
    fn test_evaluate_complex() {
        // Parse and evaluate: (A && B) || !C
        let result = parse_expr("(A && B) || !C");
        assert!(result.is_ok());
        let (_, expr) = result.unwrap();

        let mut defines = HashMap::new();
        defines.insert("A".to_string(), "1".to_string());
        defines.insert("B".to_string(), "1".to_string());
        defines.insert("C".to_string(), "0".to_string());

        assert!(evaluate(&expr, &defines, &make_all_defined(&defines)).unwrap());
    }

    #[test]
    fn test_parse_defined_with_parens() {
        let result = parse_expr("defined(FOO)");
        assert!(result.is_ok());
        let (_, expr) = result.unwrap();
        assert_eq!(expr, Expr::Defined("FOO".to_string()));
    }

    #[test]
    fn test_parse_defined_without_parens() {
        let result = parse_expr("defined FOO");
        assert!(result.is_ok());
        let (_, expr) = result.unwrap();
        assert_eq!(expr, Expr::Defined("FOO".to_string()));
    }

    #[test]
    fn test_parse_defined_with_spaces() {
        let result = parse_expr("defined ( FOO )");
        assert!(result.is_ok());
        let (_, expr) = result.unwrap();
        assert_eq!(expr, Expr::Defined("FOO".to_string()));
    }

    #[test]
    fn test_evaluate_defined_true() {
        let expr = Expr::Defined("DEBUG".to_string());
        let mut defines = HashMap::new();
        defines.insert("DEBUG".to_string(), "1".to_string());
        assert!(evaluate(&expr, &defines, &make_all_defined(&defines)).unwrap());
    }

    #[test]
    fn test_evaluate_defined_false() {
        let expr = Expr::Defined("DEBUG".to_string());
        let defines = HashMap::new();
        assert!(!evaluate(&expr, &defines, &make_all_defined(&defines)).unwrap());
    }

    #[test]
    fn test_evaluate_defined_with_empty_value() {
        let expr = Expr::Defined("EMPTY".to_string());
        let mut defines = HashMap::new();
        defines.insert("EMPTY".to_string(), "".to_string());
        assert!(evaluate(&expr, &defines, &make_all_defined(&defines)).unwrap());
    }

    #[test]
    fn test_parse_defined_in_and_expression() {
        let result = parse_expr("defined(FOO) && defined(BAR)");
        assert!(result.is_ok());
        let (_, expr) = result.unwrap();
        assert_eq!(
            expr,
            Expr::And(
                Box::new(Expr::Defined("FOO".to_string())),
                Box::new(Expr::Defined("BAR".to_string()))
            )
        );
    }

    #[test]
    fn test_parse_defined_in_or_expression() {
        let result = parse_expr("defined(FOO) || defined(BAR)");
        assert!(result.is_ok());
        let (_, expr) = result.unwrap();
        assert_eq!(
            expr,
            Expr::Or(
                Box::new(Expr::Defined("FOO".to_string())),
                Box::new(Expr::Defined("BAR".to_string()))
            )
        );
    }

    #[test]
    fn test_parse_not_defined() {
        let result = parse_expr("!defined(FOO)");
        assert!(result.is_ok());
        let (_, expr) = result.unwrap();
        assert_eq!(expr, Expr::Not(Box::new(Expr::Defined("FOO".to_string()))));
    }

    #[test]
    fn test_evaluate_defined_and_both_true() {
        let result = parse_expr("defined(FOO) && defined(BAR)");
        assert!(result.is_ok());
        let (_, expr) = result.unwrap();

        let mut defines = HashMap::new();
        defines.insert("FOO".to_string(), "1".to_string());
        defines.insert("BAR".to_string(), "1".to_string());

        assert!(evaluate(&expr, &defines, &make_all_defined(&defines)).unwrap());
    }

    #[test]
    fn test_evaluate_defined_and_one_false() {
        let result = parse_expr("defined(FOO) && defined(BAR)");
        assert!(result.is_ok());
        let (_, expr) = result.unwrap();

        let mut defines = HashMap::new();
        defines.insert("FOO".to_string(), "1".to_string());

        assert!(!evaluate(&expr, &defines, &make_all_defined(&defines)).unwrap());
    }

    #[test]
    fn test_evaluate_defined_or_one_true() {
        let result = parse_expr("defined(FOO) || defined(BAR)");
        assert!(result.is_ok());
        let (_, expr) = result.unwrap();

        let mut defines = HashMap::new();
        defines.insert("FOO".to_string(), "1".to_string());

        assert!(evaluate(&expr, &defines, &make_all_defined(&defines)).unwrap());
    }

    #[test]
    fn test_evaluate_defined_or_both_false() {
        let result = parse_expr("defined(FOO) || defined(BAR)");
        assert!(result.is_ok());
        let (_, expr) = result.unwrap();

        let defines = HashMap::new();

        assert!(!evaluate(&expr, &defines, &make_all_defined(&defines)).unwrap());
    }

    #[test]
    fn test_parse_complex_with_defined() {
        let result = parse_expr("(defined(FOO) && BAR > 0) || !defined(BAZ)");
        assert!(result.is_ok());
    }

    #[test]
    fn test_evaluate_complex_with_defined() {
        let result = parse_expr("(defined(FOO) && BAR > 0) || !defined(BAZ)");
        assert!(result.is_ok());
        let (_, expr) = result.unwrap();

        let mut defines = HashMap::new();
        defines.insert("FOO".to_string(), "1".to_string());
        defines.insert("BAR".to_string(), "5".to_string());

        assert!(evaluate(&expr, &defines, &make_all_defined(&defines)).unwrap());
    }

    #[test]
    fn test_evaluate_defined_with_identifier_value() {
        let result = parse_expr("defined(VERSION) && VERSION == 2");
        assert!(result.is_ok());
        let (_, expr) = result.unwrap();

        let mut defines = HashMap::new();
        defines.insert("VERSION".to_string(), "2".to_string());

        assert!(evaluate(&expr, &defines, &make_all_defined(&defines)).unwrap());
    }

    #[test]
    fn test_evaluate_defined_with_macro_only() {
        // Test that a macro name (not in defines) is recognized when in all_defined_names
        let expr = Expr::Defined("MACRO_FOO".to_string());
        let defines = HashMap::new();
        let mut all_defined_names = HashSet::new();
        all_defined_names.insert("MACRO_FOO".to_string());

        assert!(evaluate(&expr, &defines, &all_defined_names).unwrap());
    }

    #[test]
    fn test_evaluate_defined_with_macro_and_define() {
        // Test that both defines and macros are recognized
        let result = parse_expr("defined(DEFINE_FOO) && defined(MACRO_BAR)");
        assert!(result.is_ok());
        let (_, expr) = result.unwrap();

        let mut defines = HashMap::new();
        defines.insert("DEFINE_FOO".to_string(), "1".to_string());

        let mut all_defined_names = HashSet::new();
        all_defined_names.insert("DEFINE_FOO".to_string());
        all_defined_names.insert("MACRO_BAR".to_string());

        assert!(evaluate(&expr, &defines, &all_defined_names).unwrap());
    }

    #[test]
    fn test_evaluate_defined_macro_not_in_all_defined() {
        // Test that a macro not in all_defined_names returns false
        let expr = Expr::Defined("UNDEFINED_MACRO".to_string());
        let defines = HashMap::new();
        let all_defined_names = HashSet::new();

        assert!(!evaluate(&expr, &defines, &all_defined_names).unwrap());
    }
}
