//! Preprocessor implementation

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::fs;
use crate::preprocessor::error::{PreprocessorError, PreprocessorResult};
use crate::preprocessor::parser::{parse_directive, Directive};

/// A preprocessor that handles C-like directives
pub struct Preprocessor {
    /// Defined symbols (from #define)
    defines: HashMap<String, String>,
    
    /// Defined macros with parameters
    macros: HashMap<String, (Vec<String>, String)>,
    
    /// Base paths for resolving includes
    base_paths: Vec<PathBuf>,
    
    /// Set of currently included files (for circular dependency detection)
    include_stack: HashSet<PathBuf>,
}

impl Preprocessor {
    /// Create a new preprocessor
    pub fn new() -> Self {
        Self {
            defines: HashMap::new(),
            macros: HashMap::new(),
            base_paths: vec![PathBuf::from(".")],
            include_stack: HashSet::new(),
        }
    }
    
    /// Create a preprocessor with a specific base path for includes
    pub fn with_base_path<P: AsRef<Path>>(base_path: P) -> Self {
        Self {
            defines: HashMap::new(),
            macros: HashMap::new(),
            base_paths: vec![base_path.as_ref().to_path_buf()],
            include_stack: HashSet::new(),
        }
    }
    
    /// Create a preprocessor with multiple base paths for includes
    pub fn with_base_paths<P: AsRef<Path>>(base_paths: impl IntoIterator<Item = P>) -> Self {
        Self {
            defines: HashMap::new(),
            macros: HashMap::new(),
            base_paths: base_paths.into_iter().map(|p| p.as_ref().to_path_buf()).collect(),
            include_stack: HashSet::new(),
        }
    }
    
    /// Add a base path for resolving includes
    pub fn add_base_path<P: AsRef<Path>>(&mut self, base_path: P) {
        self.base_paths.push(base_path.as_ref().to_path_buf());
    }
    
    /// Add a predefined symbol
    pub fn define(&mut self, name: impl Into<String>, value: impl Into<String>) {
        self.defines.insert(name.into(), value.into());
    }
    
    /// Check if a symbol is defined
    pub fn is_defined(&self, name: &str) -> bool {
        self.defines.contains_key(name) || self.macros.contains_key(name)
    }
    
    /// Process source code
    pub fn process(&mut self, source: &str) -> PreprocessorResult<String> {
        let mut output = String::new();
        let mut lines = source.lines().enumerate();
        let mut conditional_stack: Vec<bool> = Vec::new();
        let mut skip_depth = 0usize;
        
        while let Some((line_num, line)) = lines.next() {
            let line_number = line_num + 1;
            let trimmed = line.trim_start();
            
            // Check if this is a preprocessor directive
            if trimmed.starts_with('#') {
                match parse_directive(trimmed) {
                    Ok((_, directive)) => {
                        match directive {
                            Directive::Define { name, value } => {
                                if skip_depth == 0 {
                                    self.defines.insert(name, value);
                                }
                            }
                            Directive::DefineMacro { name, params, body } => {
                                if skip_depth == 0 {
                                    self.macros.insert(name, (params, body));
                                }
                            }
                            Directive::IfDef { name } => {
                                let is_defined = self.is_defined(&name);
                                conditional_stack.push(is_defined);
                                if !is_defined || skip_depth > 0 {
                                    skip_depth += 1;
                                }
                            }
                            Directive::IfNDef { name } => {
                                let is_not_defined = !self.is_defined(&name);
                                conditional_stack.push(is_not_defined);
                                if !is_not_defined || skip_depth > 0 {
                                    skip_depth += 1;
                                }
                            }
                            Directive::EndIf => {
                                if conditional_stack.is_empty() {
                                    return Err(PreprocessorError::ParseError {
                                        line: line_number,
                                        message: "Unexpected #endif without matching #ifdef or #ifndef".to_string(),
                                    });
                                }
                                let condition = conditional_stack.pop().unwrap();
                                if !condition || skip_depth > 1 {
                                    skip_depth = skip_depth.saturating_sub(1);
                                }
                            }
                            Directive::Include { path } => {
                                if skip_depth == 0 {
                                    let included_content = self.process_include(&path, line_number)?;
                                    output.push_str(&included_content);
                                    output.push('\n');
                                }
                            }
                        }
                    }
                    Err(_) => {
                        return Err(PreprocessorError::ParseError {
                            line: line_number,
                            message: format!("Failed to parse directive: {}", trimmed),
                        });
                    }
                }
            } else if skip_depth == 0 {
                // Process line for macro/define substitutions
                let processed_line = self.substitute_defines(line)?;
                output.push_str(&processed_line);
                output.push('\n');
            }
        }
        
        // Check for unclosed conditionals
        if !conditional_stack.is_empty() {
            return Err(PreprocessorError::ParseError {
                line: 0,
                message: format!("{} unclosed conditional(s) (#ifdef/#ifndef without #endif)", conditional_stack.len()),
            });
        }
        
        Ok(output)
    }
    
    /// Process an include directive
    fn process_include(&mut self, path: &str, _line: usize) -> PreprocessorResult<String> {
        // Try to resolve the include path from all base paths
        let mut resolved_path = None;
        let mut last_error = None;
        
        for base_path in &self.base_paths {
            let candidate_path = base_path.join(path);
            
            // Check for circular dependencies
            if self.include_stack.contains(&candidate_path) {
                return Err(PreprocessorError::CircularDependency {
                    path: path.to_string(),
                    chain: self.include_stack.iter().map(|p| p.to_string_lossy().to_string()).collect(),
                });
            }
            
            // Try to read the file
            match fs::read_to_string(&candidate_path) {
                Ok(content) => {
                    resolved_path = Some((candidate_path, content));
                    break;
                }
                Err(e) => {
                    last_error = Some(e);
                }
            }
        }
        
        // If no file was found in any base path, return an error
        let (resolved_path, content) = resolved_path.ok_or_else(|| {
            let searched_paths: Vec<String> = self.base_paths
                .iter()
                .map(|bp| bp.join(path).to_string_lossy().to_string())
                .collect();
            let message = if let Some(err) = last_error {
                format!("{} (searched in: {})", err, searched_paths.join(", "))
            } else {
                format!("File not found in any base path (searched: {})", searched_paths.join(", "))
            };
            PreprocessorError::IoError {
                path: path.to_string(),
                message,
            }
        })?;
        
        // Add to include stack
        self.include_stack.insert(resolved_path.clone());
        
        // Process the included file
        let processed = self.process(&content)?;
        
        // Remove from include stack
        self.include_stack.remove(&resolved_path);
        
        Ok(processed)
    }
    
    /// Substitute defines and macros in a line
    fn substitute_defines(&self, line: &str) -> PreprocessorResult<String> {
        let mut result = line.to_string();
        let mut changed = true;
        let mut iterations = 0;
        const MAX_ITERATIONS: usize = 100; // Prevent infinite loops
        
        // Keep applying substitutions until no more changes occur
        while changed && iterations < MAX_ITERATIONS {
            changed = false;
            
            // First, substitute simple defines
            for (name, value) in &self.defines {
                // Simple word boundary replacement
                let new_result = self.replace_word(&result, name, value);
                if new_result != result {
                    changed = true;
                    result = new_result;
                }
            }
            
            // Then, substitute macros
            for (name, (params, body)) in &self.macros {
                let new_result = self.substitute_macro(&result, name, params, body)?;
                if new_result != result {
                    changed = true;
                    result = new_result;
                }
            }
            
            iterations += 1;
        }
        
        Ok(result)
    }
    
    /// Replace whole word occurrences
    fn replace_word(&self, text: &str, word: &str, replacement: &str) -> String {
        let mut result = String::new();
        let mut chars = text.chars().peekable();
        let mut current_word = String::new();
        
        while let Some(ch) = chars.next() {
            if ch.is_alphanumeric() || ch == '_' {
                current_word.push(ch);
            } else {
                if !current_word.is_empty() {
                    if current_word == word {
                        result.push_str(replacement);
                    } else {
                        result.push_str(&current_word);
                    }
                    current_word.clear();
                }
                result.push(ch);
            }
        }
        
        // Handle remaining word
        if !current_word.is_empty() {
            if current_word == word {
                result.push_str(replacement);
            } else {
                result.push_str(&current_word);
            }
        }
        
        result
    }
    
    /// Substitute a macro invocation
    fn substitute_macro(
        &self,
        text: &str,
        name: &str,
        params: &[String],
        body: &str,
    ) -> PreprocessorResult<String> {
        // Find macro invocations: NAME(args)
        let mut result = String::new();
        let mut i = 0;
        let chars: Vec<char> = text.chars().collect();
        
        while i < chars.len() {
            // Check if we're at the start of a macro name
            if i + name.len() <= chars.len() {
                let potential_name: String = chars[i..i + name.len()].iter().collect();
                
                if potential_name == name {
                    // Check if preceded by word boundary
                    let prev_is_boundary = i == 0 || !chars[i - 1].is_alphanumeric() && chars[i - 1] != '_';
                    
                    // Check if followed by '('
                    let mut j = i + name.len();
                    while j < chars.len() && chars[j].is_whitespace() {
                        j += 1;
                    }
                    
                    if prev_is_boundary && j < chars.len() && chars[j] == '(' {
                        // Parse macro arguments
                        let (args, end_pos) = self.parse_macro_args(&chars, j)?;
                        
                        if args.len() != params.len() {
                            return Err(PreprocessorError::InvalidMacro {
                                message: format!(
                                    "Macro {} expects {} arguments, got {}",
                                    name,
                                    params.len(),
                                    args.len()
                                ),
                                line: 0,
                            });
                        }
                        
                        // Substitute parameters in body
                        let mut expanded = body.to_string();
                        for (param, arg) in params.iter().zip(args.iter()) {
                            expanded = self.replace_word(&expanded, param, arg);
                        }
                        
                        result.push_str(&expanded);
                        i = end_pos;
                        continue;
                    }
                }
            }
            
            result.push(chars[i]);
            i += 1;
        }
        
        Ok(result)
    }
    
    /// Parse macro arguments from character array
    fn parse_macro_args(&self, chars: &[char], start: usize) -> PreprocessorResult<(Vec<String>, usize)> {
        let mut args = Vec::new();
        let mut current_arg = String::new();
        let mut depth = 0;
        let mut i = start;
        
        // Skip the opening '('
        if i >= chars.len() || chars[i] != '(' {
            return Err(PreprocessorError::InvalidMacro {
                message: "Expected '(' after macro name".to_string(),
                line: 0,
            });
        }
        i += 1;
        
        while i < chars.len() {
            match chars[i] {
                '(' => {
                    depth += 1;
                    current_arg.push('(');
                }
                ')' => {
                    if depth == 0 {
                        // End of arguments
                        if !current_arg.trim().is_empty() || !args.is_empty() {
                            args.push(current_arg.trim().to_string());
                        }
                        return Ok((args, i + 1));
                    } else {
                        depth -= 1;
                        current_arg.push(')');
                    }
                }
                ',' if depth == 0 => {
                    args.push(current_arg.trim().to_string());
                    current_arg.clear();
                }
                ch => {
                    current_arg.push(ch);
                }
            }
            i += 1;
        }
        
        Err(PreprocessorError::InvalidMacro {
            message: "Unclosed macro argument list".to_string(),
            line: 0,
        })
    }
}

impl Default for Preprocessor {
    fn default() -> Self {
        Self::new()
    }
}
