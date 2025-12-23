# pbrt-ui

A graphical user interface for [PBRT](https://www.pbrt.org/) – a physically based renderer.  
**pbrt-ui** lets you interactively configure scenes, visualize, and preprocess rendering inputs, making PBRT easier and more accessible for artists, researchers, and tinkerers.

![PBRT-UI Screenshot](https://github.com/user-attachments/assets/bdf9b2f4-0327-4950-9485-18c1b749d04f)

## Features

- Scene import, editing, and visualization
- Interactive parameter tweaking and scene graph navigation
- Integrated C-like Preprocessor (written in Rust) for macros, conditionals, and file inclusion
- Supports PBRT file format and extensions
- Real-time/pre-render previews
- Error reporting and validation for scene descriptions

### Preprocessor Highlights

The built-in preprocessor enables powerful macro expansion, conditional compilation, and file inclusion in your PBRT scenes. See [`src/preprocessor/README.md`](src/preprocessor/README.md) for full technical documentation.

**Features include:**
- `#define` for constants and macros (with parameters)
- Conditionals: `#ifdef`, `#ifndef`, `#if`, `#else`, `#elif`
- File inclusion with circular dependency checks
- Detailed error reporting and unit-tested reliability

## Prerequisites

- Rust (for building the preprocessor; see [rust-lang.org](https://www.rust-lang.org/))
- [PBRT](https://www.pbrt.org/) (external renderer, for final renders)
- Node.js & npm (if frontend is web-based)
- OS: Windows, Linux, or macOS

## Installation

```sh
git clone https://github.com/ototoi/pbrt-ui.git
cd pbrt-ui
# Build instructions depend on your stack:
cargo build         # Build Rust components (preprocessor, etc.)
npm install         # (If using a frontend framework)
npm run build
```

## Usage

1. Launch PBRT-UI:

    ```sh
    # Build and run:
    cargo run --bin pbrt-ui
    # Or run the built binary:
    ./target/debug/pbrt-ui
    ```

2. Open or create a PBRT scene.
3. Edit scene graph, geometry, materials, and parameters visually.
4. Use the integrated preprocessor for advanced configuration; see examples below.

### Preprocessor Example

```c
#define WIDTH 800
#define HEIGHT 600

#ifdef DEBUG
#define DEBUG_MODE 1
#endif

#include "common.pbrt"
```

## Contribution

Contributions are welcome!  
Feel free to open issues or submit pull requests. For major changes, please open an issue first to discuss your ideas.

## License

This project is licensed under the Apache License 2.0. See [LICENSE](LICENSE) for details.

## References

- [PBRT](https://www.pbrt.org/)
- See [`src/preprocessor/README.md`](src/preprocessor/README.md) for in-depth documentation on the preprocessor.
