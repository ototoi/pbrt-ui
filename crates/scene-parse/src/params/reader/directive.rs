use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Arity {
    /// No positional tokens.
    None,
    /// Exactly `n` quoted tokens.
    Quoted(usize),
    /// Zero to `n` quoted tokens, stopping at the first other token:
    /// MediumInterface's second name is optional (pbrt-v4 parser.cpp
    /// binds inside = outside when it is absent).
    QuotedUpTo(usize),
    /// Zero or one quoted token (`ObjectBegin [name]`).
    OptionalQuoted,
    /// Exactly `n` numeric tokens.
    Numbers(usize),
    /// Exactly `n` numeric tokens, optionally enclosed in brackets.
    Matrix(usize),
    /// Exactly one bare word (`ActiveTransform Start`).
    Word,
    /// `Option "name" value`.
    Option,
    /// Unknown directive: parameters only.
    Unknown,
}

fn arity_of(name: &str) -> Arity {
    match name {
        "WorldBegin" | "WorldEnd" | "AttributeBegin" | "AttributeEnd" | "TransformBegin"
        | "TransformEnd" | "ObjectEnd" | "ReverseOrientation" | "Identity" | "Attribute" => {
            Arity::None
        }
        "Include" | "Import" | "ColorSpace" | "Shape" | "Material" | "NamedMaterial"
        | "MakeNamedMaterial" | "MakeNamedMedium" | "ObjectInstance" | "CoordinateSystem"
        | "CoordSysTransform" | "Camera" | "Film" | "Sampler" | "Integrator" | "PixelFilter"
        | "Accelerator" | "LightSource" | "AreaLightSource" | "ParamSet" => Arity::Quoted(1),
        "MediumInterface" => Arity::QuotedUpTo(2),
        "Texture" => Arity::Quoted(3),
        "ObjectBegin" => Arity::OptionalQuoted,
        "ActiveTransform" => Arity::Word,
        "Translate" | "Scale" => Arity::Numbers(3),
        "Rotate" => Arity::Numbers(4),
        "LookAt" => Arity::Numbers(9),
        "Transform" | "ConcatTransform" => Arity::Matrix(16),
        "TransformTimes" => Arity::Numbers(2),
        "Option" => Arity::Option,
        _ => Arity::Unknown,
    }
}

/// Reads directives from a lexer, with a small push-back buffer so the parser
/// can decide where one directive ends and the next begins.
impl<'lexer, 'source> DirectiveReader<'lexer, 'source> {
    /// Next directive, or `None` at end of input.
    pub fn read_directive(&mut self) -> Result<Option<Directive>, SceneParseError> {
        self.consumed.clear();
        self.taken = 0;
        self.last_end = 0;
        let Some(head) = self.take()? else {
            return Ok(None);
        };
        let Some(name) = head.token.word() else {
            return Err(SceneParseError::syntax(
                &head.path,
                head.line,
                head.column,
                format!("expected a directive name, found {}", head.token.describe()),
            ));
        };
        let name = name.to_string();
        let mut directive = Directive {
            offset: head.offset,
            length: 0,
            positional_tokens: Vec::new(),
            name: name.clone(),
            path: head.path.to_string(),
            line: head.line,
            column: head.column,
            positional: Vec::new(),
            parameters: Vec::new(),
        };
        match arity_of(&name) {
            Arity::None | Arity::Unknown => {}
            Arity::Quoted(count) => {
                for _ in 0..count {
                    match self.peek()? {
                        Some(located) if located.token.quoted().is_some() => {
                            let located = self.take()?.expect("peeked");
                            directive
                                .positional
                                .push(Value::Text(located.token.quoted().unwrap().to_string()));
                        }
                        Some(located) => {
                            return Err(SceneParseError::syntax(
                                &located.path,
                                located.line,
                                located.column,
                                format!(
                                    "{name} expects {} quoted positional value(s), found {}",
                                    count,
                                    located.token.describe()
                                ),
                            ));
                        }
                        None => {
                            return Err(SceneParseError::syntax(
                                &directive.path,
                                directive.line,
                                directive.column,
                                format!(
                                    "{name} expects {} quoted positional value(s), found end of file",
                                    count
                                ),
                            ));
                        }
                    }
                }
            }
            Arity::QuotedUpTo(count) => {
                for _ in 0..count {
                    match self.peek()? {
                        Some(located) if located.token.quoted().is_some() => {
                            let located = self.take()?.expect("peeked");
                            directive
                                .positional
                                .push(Value::Text(located.token.quoted().unwrap().to_string()));
                        }
                        _ => break,
                    }
                }
            }
            Arity::OptionalQuoted => {
                if let Some(located) = self.peek()?
                    && let Some(text) = located.token.quoted()
                {
                    let text = text.to_string();
                    self.take()?;
                    directive.positional.push(Value::Text(text));
                }
            }
            Arity::Numbers(count) => {
                for _ in 0..count {
                    let located = self.peek()?.ok_or_else(|| {
                        SceneParseError::syntax(
                            &directive.path,
                            directive.line,
                            directive.column,
                            format!("{name} expects {count} numeric positional values"),
                        )
                    })?;
                    let Some(value) = located.token.numeric_value() else {
                        return Err(SceneParseError::syntax(
                            &located.path,
                            located.line,
                            located.column,
                            format!(
                                "{name} expects {count} numeric positional values, found {}",
                                located.token.describe()
                            ),
                        ));
                    };
                    self.take()?;
                    directive.positional.push(Value::Number(value));
                }
            }
            Arity::Matrix(count) => {
                let bracketed = self
                    .peek()?
                    .is_some_and(|located| matches!(located.token, Token::OpenBracket));
                if bracketed {
                    self.take()?;
                }
                for _ in 0..count {
                    let located = self.peek()?.ok_or_else(|| {
                        SceneParseError::syntax(
                            &directive.path,
                            directive.line,
                            directive.column,
                            format!("{name} expects {count} numeric matrix values"),
                        )
                    })?;
                    let Some(value) = located.token.numeric_value() else {
                        return Err(SceneParseError::syntax(
                            &located.path,
                            located.line,
                            located.column,
                            format!(
                                "{name} expects {count} numeric matrix values, found {}",
                                located.token.describe()
                            ),
                        ));
                    };
                    self.take()?;
                    directive.positional.push(Value::Number(value));
                }
                if bracketed {
                    let Some(located) = self.take()? else {
                        return Err(SceneParseError::syntax(
                            &directive.path,
                            directive.line,
                            directive.column,
                            format!("{name} has an unterminated matrix"),
                        ));
                    };
                    if !matches!(located.token, Token::CloseBracket) {
                        return Err(SceneParseError::syntax(
                            &located.path,
                            located.line,
                            located.column,
                            format!("{name} expects ] after {count} matrix values"),
                        ));
                    }
                }
            }
            Arity::Word => {
                let located = self.peek()?.ok_or_else(|| {
                    SceneParseError::syntax(
                        &directive.path,
                        directive.line,
                        directive.column,
                        format!("{name} expects a positional word"),
                    )
                })?;
                let Some(word) = located.token.word() else {
                    return Err(SceneParseError::syntax(
                        &located.path,
                        located.line,
                        located.column,
                        format!(
                            "{name} expects a positional word, found {}",
                            located.token.describe()
                        ),
                    ));
                };
                let word = word.to_string();
                self.take()?;
                directive.positional.push(Value::Text(word));
            }
            Arity::Option => {
                let located = self.take()?.expect("peeked");
                match located.token.quoted() {
                    Some(text) => directive.positional.push(Value::Text(text.to_string())),
                    None => {
                        return Err(SceneParseError::syntax(
                            &located.path,
                            located.line,
                            located.column,
                            format!(
                                "Option expects a quoted option name, found {}",
                                located.token.describe()
                            ),
                        ));
                    }
                }
                if let Some(located) = self.peek()?
                    && !matches!(located.token, Token::OpenBracket)
                {
                    let value = Value::from_token(&located.token);
                    self.take()?;
                    directive.positional.push(value);
                }
                if self
                    .peek()?
                    .is_some_and(|v| matches!(v.token, Token::OpenBracket))
                {
                    self.take()?;
                    loop {
                        let Some(located) = self.take()? else {
                            return Err(SceneParseError::syntax(
                                &directive.path,
                                directive.line,
                                directive.column,
                                "unterminated Option value list",
                            ));
                        };
                        if matches!(located.token, Token::CloseBracket) {
                            break;
                        }
                        directive.positional.push(Value::from_token(&located.token));
                    }
                }
            }
        }

        directive.positional_tokens = self
            .consumed
            .iter()
            .skip(1)
            .filter(|t| !matches!(t.token, Token::OpenBracket | Token::CloseBracket))
            .map(|t| t.lexeme.clone())
            .collect();
        self.read_parameters(&mut directive)?;
        // `last_end` includes the parameter values the reader never keeps;
        // `consumed` stops at the positionals.
        directive.length = self.last_end - directive.offset;
        Ok(Some(directive))
    }
}
