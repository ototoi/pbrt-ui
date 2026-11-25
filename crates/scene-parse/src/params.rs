//! Directive structure: positional arity plus the `"type" "name" [ values ]`
//! parameter vector.
//!
//! PBRT parses positionals *per directive* (a directive such as
//! `Shape "plymesh" "string filename" [ ... ]` must not swallow `"plymesh"` as
//! a parameter), so the arity is a table rather than a generic lookahead.

use std::collections::VecDeque;

use crate::error::SceneParseError;
use crate::lexer::{Lexer, Located, Token};

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Number(f64),
    Text(String),
}

/// Source label used when classifying single emitted lexemes; purely
/// diagnostic, never surfaced because these lexemes cannot fail to lex.
const CLASSIFY_SOURCE_PATH: &str = "preview://token";

impl Value {
    /// Classify one emitted lexeme by running the real lexer on it, keeping
    /// the direct path and the text path on identical token rules.
    /// Only single-token lexemes reach here; anything else is text.
    pub fn classify(lexeme: &str) -> Value {
        let mut lexer = crate::lexer::Lexer::new(CLASSIFY_SOURCE_PATH, lexeme);
        match lexer.next_token() {
            Ok(Some(located)) if located.end_offset == lexeme.len() as u64 => {
                Self::from_token(&located.token)
            }
            _ => Value::Text(lexeme.to_string()),
        }
    }

    pub fn as_number(&self) -> Option<f64> {
        match self {
            Value::Number(value) => Some(*value),
            Value::Text(_) => None,
        }
    }

    pub fn as_text(&self) -> Option<&str> {
        match self {
            Value::Text(text) => Some(text),
            Value::Number(_) => None,
        }
    }

    /// Numeric view of the value; `"inf"` / `"nan"` strings count as numbers.
    pub fn as_float(&self) -> Option<f64> {
        match self {
            Value::Number(value) => Some(*value),
            Value::Text(text) => crate::lexer::special_number(text),
        }
    }

    fn from_token(token: &Token) -> Value {
        match token {
            Token::Number(value) => Value::Number(*value),
            Token::Quoted(text) | Token::Word(text) => Value::Text(text.clone()),
            Token::OpenBracket | Token::CloseBracket => Value::Text(token.describe()),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Parameter {
    pub value_type: String,
    pub name: String,
    pub values: Vec<Value>,
    pub value_tokens: Vec<String>,
    pub line: u32,
    pub column: u32,
}

impl Parameter {
    /// PBRT parameter names are case sensitive (`P`, `indices`, `uv`); scenes
    /// in the wild are not always consistent, so lookups ignore case.
    pub fn matches(&self, name: &str) -> bool {
        self.name.eq_ignore_ascii_case(name)
    }
}

#[derive(Clone, Debug)]
pub struct Directive {
    pub offset: u64,
    pub length: u64,
    pub positional_tokens: Vec<String>,
    pub name: String,
    pub path: String,
    pub line: u32,
    pub column: u32,
    pub positional: Vec<Value>,
    pub parameters: Vec<Parameter>,
}

impl Directive {
    pub fn params(&self) -> Params<'_> {
        Params { directive: self }
    }

    pub fn positional_text(&self, index: usize) -> Option<&str> {
        self.positional.get(index).and_then(Value::as_text)
    }

    pub fn positional_float(&self, index: usize) -> Option<f64> {
        self.positional.get(index).and_then(Value::as_float)
    }
}

/// Convenience accessors over a directive's parameter vector.
pub struct Params<'directive> {
    directive: &'directive Directive,
}

mod accessors;
mod reader;
pub use reader::{DirectiveReader, SkippedInput};

#[cfg(test)]
mod tests;
