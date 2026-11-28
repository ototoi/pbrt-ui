use super::*;
mod directive;

/// Input discarded by [`DirectiveReader::recover`], from the start of the
/// directive that failed to parse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SkippedInput {
    pub offset: u64,
    pub length: u64,
    pub line: u32,
    pub column: u32,
}

/// Reads directives with bounded token tracking and a small push-back buffer.
pub struct DirectiveReader<'lexer, 'source> {
    lexer: &'lexer mut Lexer<'source>,
    peeked: VecDeque<Located>,
    /// Head and positional tokens of the directive under the cursor, for
    /// `recover`'s span and `positional_tokens`. Parameter values never
    /// land here: a bulk density array can hold millions of tokens and
    /// recovery needs only the span, which `last_end` keeps.
    consumed: Vec<Located>,
    /// End offset of the last token taken, tracked or not.
    last_end: u64,
    /// Every token taken this directive, including the parameter values
    /// `consumed` does not keep: the token budget counts a bulk density
    /// array the reader never stores.
    taken: usize,
    token_limit: usize,
}

impl<'lexer, 'source> DirectiveReader<'lexer, 'source> {
    pub fn new(lexer: &'lexer mut Lexer<'source>) -> Self {
        Self {
            lexer,
            peeked: VecDeque::new(),
            consumed: Vec::new(),
            last_end: 0,
            taken: 0,
            token_limit: usize::MAX,
        }
    }

    pub fn with_token_limit(lexer: &'lexer mut Lexer<'source>, token_limit: usize) -> Self {
        let mut reader = Self::new(lexer);
        reader.token_limit = token_limit;
        reader
    }

    pub fn path(&self) -> String {
        self.lexer.path().to_string()
    }

    pub fn line(&self) -> u32 {
        self.lexer.line()
    }

    /// After a syntax error, discard input up to the next token that can
    /// start a directive (a bare word beginning with an ASCII uppercase
    /// letter), so reading can continue. Lexer errors inside the discarded
    /// input are skipped too; an error that consumes no input is returned.
    pub fn recover(&mut self) -> Result<SkippedInput, SceneParseError> {
        let start = self.consumed.first().cloned();
        let mut end = Some(self.last_end);
        loop {
            let before = self.lexer.offset();
            match self.peek() {
                Ok(None) => break,
                Ok(Some(token))
                    if token
                        .token
                        .word()
                        .is_some_and(|w| w.starts_with(|c: char| c.is_ascii_uppercase())) =>
                {
                    break;
                }
                Ok(Some(_)) => {
                    end = self.take_untracked()?.map(|t| t.end_offset);
                }
                Err(SceneParseError::Syntax { .. }) if self.lexer.offset() > before => {
                    end = Some(self.lexer.offset());
                }
                Err(error) => return Err(error),
            }
        }
        let (offset, line, column) = match &start {
            Some(token) => (token.offset, token.line, token.column),
            None => (self.lexer.offset(), self.lexer.line(), self.lexer.column()),
        };
        let end = end.unwrap_or(offset).max(offset);
        self.consumed.clear();
        Ok(SkippedInput {
            offset,
            length: end - offset,
            line,
            column,
        })
    }

    fn peek(&mut self) -> Result<Option<Located>, SceneParseError> {
        if self.peeked.is_empty()
            && let Some(token) = self.lexer.next_token()?
        {
            self.peeked.push_back(token);
        }
        Ok(self.peeked.front().cloned())
    }

    /// Take the next token and record it in `consumed` (the directive's
    /// head and positionals).
    fn take(&mut self) -> Result<Option<Located>, SceneParseError> {
        let token = self.take_untracked()?;
        if let Some(token) = &token {
            self.consumed.push(token.clone());
        }
        Ok(token)
    }

    /// Take without recording: parameter value tokens can run to millions
    /// (a medium's density array); their span lives in `last_end` and the
    /// values in the Directive, so the reader never keeps them.
    fn take_untracked(&mut self) -> Result<Option<Located>, SceneParseError> {
        let _ = self.peek()?; // fill the push-back buffer
        let token = self.peeked.pop_front();
        if let Some(token) = &token {
            if self.taken >= self.token_limit {
                return Err(SceneParseError::ResourceExhausted {
                    message: format!(
                        "more than {} tokens in one directive (import limit)",
                        self.token_limit
                    ),
                });
            }
            self.taken += 1;
            self.last_end = token.end_offset;
        }
        Ok(token)
    }

    fn read_parameters(&mut self, directive: &mut Directive) -> Result<(), SceneParseError> {
        loop {
            let Some(first) = self.peek()? else {
                return Ok(());
            };
            let Some(declaration) = first.token.quoted() else {
                return Ok(());
            };
            let mut parts = declaration.trim_start().splitn(2, char::is_whitespace);
            let Some(value_type) = parts.next().filter(|part| !part.is_empty()) else {
                return Ok(());
            };
            let Some(name) = parts.next().map(str::trim).filter(|part| !part.is_empty()) else {
                return Ok(());
            };
            let type_located = self.take_untracked()?.expect("peeked");
            let (values, value_tokens) = self.read_parameter_values(directive)?;
            directive.parameters.push(Parameter {
                value_type: value_type.to_string(),
                name: name.to_string(),
                values,
                value_tokens,
                line: type_located.line,
                column: type_located.column,
            });
        }
    }

    fn read_parameter_values(
        &mut self,
        directive: &Directive,
    ) -> Result<(Vec<Value>, Vec<String>), SceneParseError> {
        let Some(located) = self.peek()? else {
            return Ok((Vec::new(), Vec::new()));
        };
        if !matches!(located.token, Token::OpenBracket) {
            // Single-value parameter without brackets, e.g. `"float fov" 47`.
            let located = self.take_untracked()?.expect("peeked");
            if matches!(located.token, Token::OpenBracket | Token::CloseBracket) {
                return Err(SceneParseError::syntax(
                    &directive.path,
                    directive.line,
                    directive.column,
                    format!(
                        "parameter values expected after \"{}\"",
                        located.token.describe()
                    ),
                ));
            }
            return Ok((
                vec![Value::from_token(&located.token)],
                vec![located.lexeme],
            ));
        }
        self.take_untracked()?; // [
        let mut values = Vec::new();
        let mut value_tokens = Vec::new();
        loop {
            let Some(located) = self.peek()? else {
                return Err(SceneParseError::syntax(
                    &directive.path,
                    directive.line,
                    directive.column,
                    "unterminated parameter value list",
                ));
            };
            match located.token {
                Token::OpenBracket => {
                    return Err(SceneParseError::syntax(
                        &located.path,
                        located.line,
                        located.column,
                        "nested parameter value lists are not supported",
                    ));
                }
                Token::CloseBracket => {
                    self.take_untracked()?;
                    return Ok((values, value_tokens));
                }
                _ => {
                    let located = self.take_untracked()?.expect("peeked");
                    values.push(Value::from_token(&located.token));
                    value_tokens.push(located.lexeme);
                }
            }
        }
    }
}
