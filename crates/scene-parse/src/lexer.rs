//! PBRT directive lexer.
//!
//! The PBRT file format is a stream of directives: a word, positional tokens,
//! and `"type" "name" [ values ]` parameter lists. This lexer only splits
//! tokens and tracks `path:line:column`; directive structure (including the
//! bracketed value lists) is handled in `params`.

use crate::error::SceneParseError;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq)]
pub enum Token {
    /// A bare word such as `Shape`, `trianglemesh`, `Start`, or `inf`.
    Word(String),
    /// A `"quoted"` string.
    Quoted(String),
    /// A numeric literal. `inf` / `nan` spellings arrive as `Word`.
    Number(f64),
    OpenBracket,
    CloseBracket,
}

impl Token {
    pub fn word(&self) -> Option<&str> {
        match self {
            Token::Word(text) => Some(text),
            _ => None,
        }
    }

    pub fn quoted(&self) -> Option<&str> {
        match self {
            Token::Quoted(text) => Some(text),
            _ => None,
        }
    }

    pub fn number(&self) -> Option<f64> {
        match self {
            Token::Number(value) => Some(*value),
            _ => None,
        }
    }

    /// A numeric value, accepting the `inf` / `nan` spellings PBRT allows.
    pub fn numeric_value(&self) -> Option<f64> {
        match self {
            Token::Number(value) => Some(*value),
            Token::Word(text) => special_number(text),
            _ => None,
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Token::Word(text) => format!("word {text}"),
            Token::Quoted(text) => format!("string \"{text}\""),
            Token::Number(value) => format!("number {value}"),
            Token::OpenBracket => "[".to_string(),
            Token::CloseBracket => "]".to_string(),
        }
    }
}

/// `inf`, `+inf`, `-inf`, `nan` are valid numeric literals in PBRT.
pub fn special_number(text: &str) -> Option<f64> {
    let (sign, rest) = match text.strip_prefix('-') {
        Some(rest) => (-1.0f64, rest),
        None => (1.0f64, text.strip_prefix('+').unwrap_or(text)),
    };
    match rest.to_ascii_lowercase().as_str() {
        "inf" | "infinity" => Some(sign * f64::INFINITY),
        "nan" => Some(f64::NAN),
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Located {
    pub token: Token,
    /// Exact UTF-8 bytes, before numeric conversion or string unescaping.
    pub lexeme: String,
    pub offset: u64,
    pub end_offset: u64,
    /// Shared source path: one directive can produce millions of tokens
    /// (a medium's density array), so the path must not be a per-token
    /// allocation.
    pub path: Arc<str>,
    pub line: u32,
    pub column: u32,
}

pub struct Lexer<'a> {
    bytes: &'a [u8],
    path: Arc<str>,
    offset: usize,
    line: u32,
    column: u32,
}

impl<'a> Lexer<'a> {
    pub fn new(path: impl Into<Arc<str>>, source: &'a str) -> Self {
        Self {
            bytes: source.as_bytes(),
            path: path.into(),
            offset: 0,
            line: 1,
            column: 1,
        }
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn line(&self) -> u32 {
        self.line
    }

    pub fn column(&self) -> u32 {
        self.column
    }

    /// Byte offset of the next unread input.
    pub fn offset(&self) -> u64 {
        self.offset as u64
    }

    fn peek_byte(&self) -> Option<u8> {
        self.bytes.get(self.offset).copied()
    }

    fn advance(&mut self) {
        match self.bytes.get(self.offset) {
            Some(b'\n') => {
                self.offset += 1;
                self.line += 1;
                self.column = 1;
            }
            Some(_) => {
                self.offset += 1;
                self.column += 1;
            }
            None => {}
        }
    }

    fn skip_trivia(&mut self) {
        loop {
            match self.peek_byte() {
                Some(b'\n') | Some(b' ') | Some(b'\t') | Some(b'\r') => self.advance(),
                Some(b'#') => {
                    while !matches!(self.peek_byte(), None | Some(b'\n')) {
                        self.advance();
                    }
                }
                _ => return,
            }
        }
    }

    /// Next token, or `None` at end of input.
    pub fn next_token(&mut self) -> Result<Option<Located>, SceneParseError> {
        self.skip_trivia();
        let offset = self.offset;
        let line = self.line;
        let column = self.column;
        let token = match self.peek_byte() {
            None => return Ok(None),
            Some(b'[') => {
                self.advance();
                Token::OpenBracket
            }
            Some(b']') => {
                self.advance();
                Token::CloseBracket
            }
            Some(b'"') => Token::Quoted(self.read_quoted()?),
            Some(byte)
                if byte.is_ascii_digit()
                    || byte == b'.'
                    || ((byte == b'-' || byte == b'+')
                        && self.bytes.get(self.offset + 1).is_some_and(|next| {
                            next.is_ascii_digit()
                                || (*next == b'.'
                                    && self
                                        .bytes
                                        .get(self.offset + 2)
                                        .is_some_and(u8::is_ascii_digit))
                        })) =>
            {
                Token::Number(self.read_number()?)
            }
            Some(_) => Token::Word(self.read_word()),
        };
        Ok(Some(Located {
            token,
            lexeme: String::from_utf8_lossy(&self.bytes[offset..self.offset]).into_owned(),
            offset: offset as u64,
            end_offset: self.offset as u64,
            path: self.path.clone(),
            line,
            column,
        }))
    }

    fn read_quoted(&mut self) -> Result<String, SceneParseError> {
        let line = self.line;
        let column = self.column;
        self.advance(); // opening quote
        let mut text = Vec::new();
        loop {
            match self.peek_byte() {
                None => {
                    return Err(SceneParseError::syntax(
                        &self.path,
                        line,
                        column,
                        "unterminated string",
                    ));
                }
                Some(b'"') => {
                    self.advance();
                    return String::from_utf8(text).map_err(|_| {
                        SceneParseError::syntax(&self.path, line, column, "invalid UTF-8 string")
                    });
                }
                Some(b'\\') => {
                    self.advance();
                    match self.peek_byte() {
                        None => {
                            return Err(SceneParseError::syntax(
                                &self.path,
                                line,
                                column,
                                "unterminated string",
                            ));
                        }
                        Some(b'n') => text.push(b'\n'),
                        Some(b't') => text.push(b'\t'),
                        Some(b'r') => text.push(b'\r'),
                        Some(escaped) => {
                            // `\"`, `\\`, and anything else keep the literal byte.
                            text.push(escaped);
                        }
                    }
                    self.advance();
                }
                Some(byte) => {
                    text.push(byte);
                    self.advance();
                }
            }
        }
    }

    fn read_number(&mut self) -> Result<f64, SceneParseError> {
        let line = self.line;
        let column = self.column;
        let start = self.offset;
        if matches!(self.peek_byte(), Some(b'-') | Some(b'+')) {
            self.advance();
        }
        let mut saw_dot = false;
        while let Some(byte) = self.peek_byte() {
            if byte.is_ascii_digit() {
                self.advance();
            } else if byte == b'.' && !saw_dot {
                saw_dot = true;
                self.advance();
            } else {
                break;
            }
        }
        // Optional exponent, e.g. `1e30`.
        if matches!(self.peek_byte(), Some(b'e') | Some(b'E')) {
            let save = self.offset;
            let save_line = self.line;
            let save_column = self.column;
            self.advance();
            if matches!(self.peek_byte(), Some(b'-') | Some(b'+')) {
                self.advance();
            }
            if self.peek_byte().is_some_and(|b| b.is_ascii_digit()) {
                while self.peek_byte().is_some_and(|byte| byte.is_ascii_digit()) {
                    self.advance();
                }
            } else {
                // Not an exponent after all; rewind so the word lexer sees it.
                self.offset = save;
                self.line = save_line;
                self.column = save_column;
            }
        }
        let text = std::str::from_utf8(&self.bytes[start..self.offset])
            .map_err(|_| SceneParseError::syntax(&self.path, line, column, "invalid number"))?;
        text.parse::<f64>().map_err(|_| {
            SceneParseError::syntax(&self.path, line, column, format!("invalid number {text}"))
        })
    }

    fn read_word(&mut self) -> String {
        let start = self.offset;
        while let Some(byte) = self.peek_byte() {
            let is_word_byte = !(byte.is_ascii_whitespace()
                || byte == b'"'
                || byte == b'['
                || byte == b']'
                || byte == b'#');
            if !is_word_byte {
                break;
            }
            self.advance();
        }
        String::from_utf8_lossy(&self.bytes[start..self.offset]).into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(source: &str) -> Vec<Token> {
        let mut lexer = Lexer::new("test.pbrt", source);
        let mut out = Vec::new();
        loop {
            let located = lexer.next_token().expect("lex");
            match located {
                Some(located) => out.push(located.token),
                None => break,
            }
        }
        out
    }

    #[test]
    fn lexes_directive_with_quoted_names_and_bracketed_values() {
        let source = "Shape \"trianglemesh\"  \"point P\" [ 0 0 0  1 0 0  0 1 0 ]\n";
        assert_eq!(
            tokens(source),
            vec![
                Token::Word("Shape".into()),
                Token::Quoted("trianglemesh".into()),
                Token::Quoted("point P".into()),
                Token::OpenBracket,
                Token::Number(0.0),
                Token::Number(0.0),
                Token::Number(0.0),
                Token::Number(1.0),
                Token::Number(0.0),
                Token::Number(0.0),
                Token::Number(0.0),
                Token::Number(1.0),
                Token::Number(0.0),
                Token::CloseBracket,
            ]
        );
    }

    #[test]
    fn comments_and_blank_lines_do_not_shift_line_numbers() {
        let mut lexer = Lexer::new("test.pbrt", "# header\n\nWorldBegin\n");
        let first = lexer.next_token().expect("lex").expect("token");
        assert_eq!(first.token, Token::Word("WorldBegin".into()));
        assert_eq!((first.line, first.column), (3, 1));
    }

    #[test]
    fn tracks_line_and_column_of_later_tokens() {
        let mut lexer = Lexer::new("test.pbrt", "Shape \"plymesh\"\n  \"string filename\"\n");
        let shape = lexer.next_token().expect("lex").expect("token");
        let name = lexer.next_token().expect("lex").expect("token");
        let ty = lexer.next_token().expect("lex").expect("token");
        assert_eq!((shape.line, shape.column), (1, 1));
        assert_eq!((name.line, name.column), (1, 7));
        assert_eq!((ty.line, ty.column), (2, 3));
    }

    #[test]
    fn reads_escapes_and_inf_literals() {
        let mut lexer = Lexer::new("test.pbrt", "\"a\\\"b\" inf -inf nan 1e30 .5");
        assert_eq!(
            lexer.next_token().expect("lex").expect("token").token,
            Token::Quoted("a\"b".into())
        );
        assert_eq!(
            lexer
                .next_token()
                .expect("lex")
                .expect("token")
                .token
                .numeric_value(),
            Some(f64::INFINITY)
        );
        assert_eq!(
            lexer
                .next_token()
                .expect("lex")
                .expect("token")
                .token
                .numeric_value(),
            Some(f64::NEG_INFINITY)
        );
        assert!(
            lexer
                .next_token()
                .expect("lex")
                .expect("token")
                .token
                .numeric_value()
                .unwrap()
                .is_nan()
        );
        assert_eq!(
            lexer.next_token().expect("lex").expect("token").token,
            Token::Number(1e30)
        );
        assert_eq!(
            lexer.next_token().expect("lex").expect("token").token,
            Token::Number(0.5)
        );
    }

    #[test]
    fn lexes_signed_fraction_without_leading_zero() {
        assert_eq!(
            tokens("-.03 +.5 -0.25"),
            vec![
                Token::Number(-0.03),
                Token::Number(0.5),
                Token::Number(-0.25),
            ]
        );
    }

    #[test]
    fn unterminated_string_is_a_syntax_error_with_location() {
        let mut lexer = Lexer::new("test.pbrt", "Film \"rgb\"  \"string filename\" [ \"a\n");
        let error = loop {
            match lexer.next_token() {
                Ok(Some(_)) => continue,
                Err(error) => break error,
                Ok(None) => panic!("unterminated string should fail"),
            }
        };
        match error {
            SceneParseError::Syntax {
                path, line, column, ..
            } => {
                assert_eq!(path, "test.pbrt");
                assert_eq!((line, column), (1, 33));
            }
            other => panic!("unexpected error: {other}"),
        }
    }
}
