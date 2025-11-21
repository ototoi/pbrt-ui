//! Error types for the preprocessor

use std::fmt;

/// Result type for preprocessor operations
pub type PreprocessorResult<T> = Result<T, PreprocessorError>;

/// Errors that can occur during preprocessing
#[derive(Debug, Clone, PartialEq)]
pub enum PreprocessorError {
    /// Parse error with line number and description
    ParseError { line: usize, message: String },

    /// File not found for include directive
    FileNotFound { path: String, line: usize },

    /// Circular dependency detected in includes
    CircularDependency { path: String, chain: Vec<String> },

    /// IO error reading file
    IoError { path: String, message: String },

    /// Undefined symbol referenced
    UndefinedSymbol { symbol: String, line: usize },

    /// Invalid macro usage
    InvalidMacro { message: String, line: usize },
}

impl fmt::Display for PreprocessorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PreprocessorError::ParseError { line, message } => {
                write!(f, "Parse error at line {}: {}", line, message)
            }
            PreprocessorError::FileNotFound { path, line } => {
                write!(f, "File not found at line {}: {}", line, path)
            }
            PreprocessorError::CircularDependency { path, chain } => {
                write!(
                    f,
                    "Circular dependency detected: {} -> {}",
                    chain.join(" -> "),
                    path
                )
            }
            PreprocessorError::IoError { path, message } => {
                write!(f, "IO error reading '{}': {}", path, message)
            }
            PreprocessorError::UndefinedSymbol { symbol, line } => {
                write!(f, "Undefined symbol '{}' at line {}", symbol, line)
            }
            PreprocessorError::InvalidMacro { message, line } => {
                write!(f, "Invalid macro at line {}: {}", line, message)
            }
        }
    }
}

impl std::error::Error for PreprocessorError {}
