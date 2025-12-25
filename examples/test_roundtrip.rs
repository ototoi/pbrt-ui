use pbrt_ui::io::pbrt::export::{SavePbrtOptions, save_pbrt};
use pbrt_ui::io::pbrt::import::load_pbrt;

fn main() {
    let input_file = "test_simple.pbrt";
    let output_file = "test_output.pbrt";

    println!("Loading {}...", input_file);
    let node = match load_pbrt(input_file) {
        Ok(n) => {
            println!("✓ Loaded successfully");
            n
        }
        Err(e) => {
            eprintln!("✗ Failed to load: {}", e);
            std::process::exit(1);
        }
    };

    println!("Saving to {}...", output_file);
    let options = SavePbrtOptions::default();
    match save_pbrt(&node, output_file, &options) {
        Ok(_) => println!("✓ Saved successfully"),
        Err(e) => {
            eprintln!("✗ Failed to save: {}", e);
            std::process::exit(1);
        }
    }

    // Read and display the output
    match std::fs::read_to_string(output_file) {
        Ok(content) => {
            println!("\n=== Output content ===");
            println!("{}", content);
        }
        Err(e) => {
            eprintln!("✗ Failed to read output: {}", e);
        }
    }
}
