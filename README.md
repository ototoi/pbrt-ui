# pbrt-ui

A graphical user interface for <a href="https://www.pbrt.org/">PBRT</a> – a physically based renderer.  
**pbrt-ui** lets you interactively configure scenes, visualize, and preprocess rendering inputs, making PBRT easier and more accessible for artists, researchers, and tinkerers.

<img src="https://github.com/user-attachments/assets/bdf9b2f4-0327-4950-9485-18c1b749d04f">

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

- Rust (for building the preprocessor; see <a href="https://www.rust-lang.org/">rust-lang.org</a>)
- <a href="https://www.pbrt.org/">PBRT</a> (external renderer, for final renders)
- Node.js &amp; npm (if frontend is web-based; clarify if necessary)
- OS: Windows, Linux, or macOS (specify your tested platforms)

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
    # Example: (Replace with your actual launch command)
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
#   info("Debug mode!")
#endif

#include "common.pbrt"
```

## Screenshot

<img src="https://github.com/user-attachments/assets/bdf9b2f4-0327-4950-9485-18c1b749d04f">

## Contribution

Contributions are welcome!  
Feel free to open issues or submit pull requests. For major changes, please open an issue first to discuss your ideas.

## License

This project is licensed under the Apache License 2.0. See [LICENSE](LICENSE) for details.

## References

- [PBRT](https://www.pbrt.org/)
- See [`src/preprocessor/README.md`](src/preprocessor/README.md) for in-depth documentation on the preprocessor.
