# Preprocessor

A C-like preprocessor written in Rust using the `nom` parser combinator library.

## Features

- **`#define`** - Define constants and macros
  - Simple defines: `#define PI 3.14159`
  - Macros with parameters: `#define MAX(a, b) ((a) > (b) ? (a) : (b))`
  - Recursive macro expansion
  
- **`#ifdef` / `#ifndef`** - Conditional compilation
  - Check if a symbol is defined
  - Support for nested conditionals
  
- **`#include`** - File inclusion
  - Include external files
  - Circular dependency detection
  - Support for both `"quoted"` and `<angled>` paths

## Usage

```rust
use pbrt_ui::preprocessor::Preprocessor;

fn main() {
    let mut preprocessor = Preprocessor::new();
    
    // Define a symbol
    preprocessor.define("DEBUG", "1");
    
    let source = r#"
        #define PI 3.14159
        #define CIRCLE_AREA(r) (PI * (r) * (r))
        
        #ifdef DEBUG
        let debug_mode = true;
        #endif
        
        let area = CIRCLE_AREA(5.0);
    "#;
    
    match preprocessor.process(source) {
        Ok(output) => println!("{}", output),
        Err(e) => eprintln!("Error: {}", e),
    }
}
```

## Examples

### Simple Constants

```
#define WIDTH 800
#define HEIGHT 600
let screen_size = vec2<f32>(WIDTH, HEIGHT);
```

Output:
```
let screen_size = vec2<f32>(800, 600);
```

### Macros with Parameters

```
#define MAX(a, b) ((a) > (b) ? (a) : (b))
#define CLAMP(v, min, max) MAX(MIN(v, max), min)

let clamped_value = CLAMP(x, 0.0, 1.0);
```

### Conditional Compilation

```
#define USE_TEXTURE
#ifdef USE_TEXTURE
fn sample_color() -> vec4<f32> {
    return textureSample(my_texture, my_sampler, uv);
}
#endif
#ifndef USE_TEXTURE
fn sample_color() -> vec4<f32> {
    return vec4<f32>(1.0, 1.0, 1.0, 1.0);
}
#endif
```

### File Inclusion

**common.h:**
```
#define PI 3.14159265359
#define TWO_PI (2.0 * PI)

struct Camera {
    position: vec3<f32>,
    direction: vec3<f32>,
}
```

**main.c:**
```
#include "common.h"

@group(0) @binding(0) var<uniform> camera: Camera;

fn get_rotation_angle() -> f32 {
    return TWO_PI;
}
```

## Error Handling

The preprocessor provides detailed error messages:

```rust
use pbrt_ui::preprocessor::{Preprocessor, PreprocessorError};

let mut preprocessor = Preprocessor::new();
let source = "#ifdef UNDEFINED\ncode\n"; // Missing #endif

match preprocessor.process(source) {
    Err(PreprocessorError::ParseError { line, message }) => {
        println!("Parse error at line {}: {}", line, message);
    }
    Err(PreprocessorError::CircularDependency { path, chain }) => {
        println!("Circular dependency: {} -> {}", chain.join(" -> "), path);
    }
    _ => {}
}
```

## API

### `Preprocessor::new()`
Create a new preprocessor with the current directory as base path.

### `Preprocessor::with_base_path(path)`
Create a preprocessor with a specific base path for resolving includes.

### `preprocessor.define(name, value)`
Add a predefined symbol before processing.

### `preprocessor.is_defined(name)`
Check if a symbol is currently defined.

### `preprocessor.process(source)`
Process source code and return the preprocessed output.

## Testing

The implementation includes comprehensive unit tests covering:

- Simple and complex defines
- Macro expansion with parameters
- Nested macro calls
- Conditional compilation
- Nested conditionals
- File inclusion
- Circular dependency detection
- Various error cases

Run tests with:
```bash
cargo test preprocessor
```

## Implementation Details

The preprocessor is implemented in several modules:

- **`parser`** - Uses `nom` parser combinators to parse directives
- **`processor`** - Main preprocessing logic with symbol table management
- **`error`** - Comprehensive error types with helpful messages
- **`tests`** - Unit tests for all features

The preprocessor supports recursive macro expansion, properly handles nested conditionals, and detects circular dependencies in includes.
