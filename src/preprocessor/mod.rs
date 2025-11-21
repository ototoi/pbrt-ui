//! Preprocessor
//!
//! A C-like preprocessor that handles:
//! - `#define` directives for constant and macro definitions
//! - `#ifdef` and `#ifndef` for conditional compilation
//! - `#include` for file inclusion with circular dependency detection

mod error;
mod parser;
mod processor;

pub use error::{PreprocessorError, PreprocessorResult};
pub use processor::Preprocessor;

#[cfg(test)]
mod tests;
