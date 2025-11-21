//! Parser combinators for preprocessor directives using nom

use nom::{
    IResult,
    branch::alt,
    bytes::complete::{tag, take_until, take_while},
    character::complete::{char, space0, space1, alphanumeric1},
    combinator::{opt, recognize, map},
    multi::{many0, separated_list0},
    sequence::{delimited, tuple},
};

/// Represents a preprocessor directive
#[derive(Debug, Clone, PartialEq)]
pub enum Directive {
    /// #define NAME VALUE
    Define { name: String, value: String },
    
    /// #define NAME(params) body
    DefineMacro { name: String, params: Vec<String>, body: String },
    
    /// #ifdef NAME
    IfDef { name: String },
    
    /// #ifndef NAME
    IfNDef { name: String },
    
    /// #endif
    EndIf,
    
    /// #include "path" or #include <path>
    Include { path: String },
}

/// Parse an identifier (alphanumeric + underscore, starting with letter or underscore)
fn identifier(input: &str) -> IResult<&str, &str> {
    recognize(tuple((
        alt((alphanumeric1, tag("_"))),
        many0(alt((alphanumeric1, tag("_")))),
    )))(input)
}

/// Parse whitespace (spaces and tabs only, not newlines)
fn ws(input: &str) -> IResult<&str, &str> {
    take_while(|c| c == ' ' || c == '\t')(input)
}

/// Parse until end of line
fn until_eol(input: &str) -> IResult<&str, &str> {
    take_while(|c| c != '\n' && c != '\r')(input)
}

/// Parse a simple #define directive
fn parse_define_simple(input: &str) -> IResult<&str, Directive> {
    let (input, _) = tag("#define")(input)?;
    let (input, _) = space1(input)?;
    let (input, name) = identifier(input)?;
    let (input, _) = ws(input)?;
    let (input, value) = until_eol(input)?;
    
    Ok((input, Directive::Define {
        name: name.to_string(),
        value: value.trim().to_string(),
    }))
}

/// Parse macro parameters like (a, b, c)
fn parse_macro_params(input: &str) -> IResult<&str, Vec<String>> {
    delimited(
        char('('),
        map(
            opt(separated_list0(
                delimited(space0, char(','), space0),
                map(identifier, |s| s.to_string())
            )),
            |opt_list| opt_list.unwrap_or_default()
        ),
        char(')')
    )(input)
}

/// Parse a macro #define directive
fn parse_define_macro(input: &str) -> IResult<&str, Directive> {
    let (input, _) = tag("#define")(input)?;
    let (input, _) = space1(input)?;
    let (input, name) = identifier(input)?;
    let (input, params) = parse_macro_params(input)?;
    let (input, _) = ws(input)?;
    let (input, body) = until_eol(input)?;
    
    Ok((input, Directive::DefineMacro {
        name: name.to_string(),
        params,
        body: body.trim().to_string(),
    }))
}

/// Parse #define (either simple or macro)
fn parse_define(input: &str) -> IResult<&str, Directive> {
    alt((parse_define_macro, parse_define_simple))(input)
}

/// Parse #ifdef directive
fn parse_ifdef(input: &str) -> IResult<&str, Directive> {
    let (input, _) = tag("#ifdef")(input)?;
    let (input, _) = space1(input)?;
    let (input, name) = identifier(input)?;
    
    Ok((input, Directive::IfDef {
        name: name.to_string(),
    }))
}

/// Parse #ifndef directive
fn parse_ifndef(input: &str) -> IResult<&str, Directive> {
    let (input, _) = tag("#ifndef")(input)?;
    let (input, _) = space1(input)?;
    let (input, name) = identifier(input)?;
    
    Ok((input, Directive::IfNDef {
        name: name.to_string(),
    }))
}

/// Parse #endif directive
fn parse_endif(input: &str) -> IResult<&str, Directive> {
    let (input, _) = tag("#endif")(input)?;
    
    Ok((input, Directive::EndIf))
}

/// Parse #include directive with quoted path
fn parse_include_quoted(input: &str) -> IResult<&str, &str> {
    delimited(char('"'), take_until("\""), char('"'))(input)
}

/// Parse #include directive with angle brackets
fn parse_include_angled(input: &str) -> IResult<&str, &str> {
    delimited(char('<'), take_until(">"), char('>'))(input)
}

/// Parse #include directive
fn parse_include(input: &str) -> IResult<&str, Directive> {
    let (input, _) = tag("#include")(input)?;
    let (input, _) = space1(input)?;
    let (input, path) = alt((parse_include_quoted, parse_include_angled))(input)?;
    
    Ok((input, Directive::Include {
        path: path.to_string(),
    }))
}

/// Parse any preprocessor directive
pub fn parse_directive(input: &str) -> IResult<&str, Directive> {
    alt((
        parse_include,
        parse_define,
        parse_ifdef,
        parse_ifndef,
        parse_endif,
    ))(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_define() {
        let input = "#define PI 3.14159";
        let result = parse_define(input);
        assert!(result.is_ok());
        let (_, directive) = result.unwrap();
        assert_eq!(
            directive,
            Directive::Define {
                name: "PI".to_string(),
                value: "3.14159".to_string(),
            }
        );
    }

    #[test]
    fn test_parse_macro_define() {
        let input = "#define MAX(a, b) ((a) > (b) ? (a) : (b))";
        let result = parse_define(input);
        assert!(result.is_ok());
        let (_, directive) = result.unwrap();
        assert_eq!(
            directive,
            Directive::DefineMacro {
                name: "MAX".to_string(),
                params: vec!["a".to_string(), "b".to_string()],
                body: "((a) > (b) ? (a) : (b))".to_string(),
            }
        );
    }

    #[test]
    fn test_parse_ifdef() {
        let input = "#ifdef DEBUG";
        let result = parse_ifdef(input);
        assert!(result.is_ok());
        let (_, directive) = result.unwrap();
        assert_eq!(
            directive,
            Directive::IfDef {
                name: "DEBUG".to_string(),
            }
        );
    }

    #[test]
    fn test_parse_ifndef() {
        let input = "#ifndef RELEASE";
        let result = parse_ifndef(input);
        assert!(result.is_ok());
        let (_, directive) = result.unwrap();
        assert_eq!(
            directive,
            Directive::IfNDef {
                name: "RELEASE".to_string(),
            }
        );
    }

    #[test]
    fn test_parse_endif() {
        let input = "#endif";
        let result = parse_endif(input);
        assert!(result.is_ok());
        let (_, directive) = result.unwrap();
        assert_eq!(directive, Directive::EndIf);
    }

    #[test]
    fn test_parse_include_quoted() {
        let input = r#"#include "shader.wgsl""#;
        let result = parse_include(input);
        assert!(result.is_ok());
        let (_, directive) = result.unwrap();
        assert_eq!(
            directive,
            Directive::Include {
                path: "shader.wgsl".to_string(),
            }
        );
    }

    #[test]
    fn test_parse_include_angled() {
        let input = "#include <common.wgsl>";
        let result = parse_include(input);
        assert!(result.is_ok());
        let (_, directive) = result.unwrap();
        assert_eq!(
            directive,
            Directive::Include {
                path: "common.wgsl".to_string(),
            }
        );
    }
}
