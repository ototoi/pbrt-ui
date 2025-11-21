//! Preprocessor
//! 
//! A C-like preprocessor that handles:
//! - `#define` directives for constant and macro definitions
//! - `#ifdef` and `#ifndef` for conditional compilation
//! - `#include` for file inclusion with circular dependency detection

mod parser;
mod processor;
mod error;

pub use processor::Preprocessor;
pub use error::{PreprocessorError, PreprocessorResult};

#[cfg(test)]
mod tests;
