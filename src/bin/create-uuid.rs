use clap::Parser;
use uuid::Uuid;

#[derive(Debug, Parser)]
#[clap(author, about = "Generate a UUID", version)]
struct Options {
    #[arg(
        short = 'f',
        long,
        default_value = "hyphenated",
        help = "Output format: hyphenated, simple, or urn"
    )]
    format: String,
}

fn main() -> Result<(), String> {
    let options = Options::parse();

    // Generate a new UUID (v4)
    let uuid = Uuid::new_v4();

    // Format and print the UUID based on the selected format
    let output = match options.format.as_str() {
        "hyphenated" => uuid.hyphenated().to_string(),
        "simple" => uuid.simple().to_string(),
        "urn" => uuid.urn().to_string(),
        _ => {
            return Err(format!(
                "Invalid format: '{}'. Must be 'hyphenated', 'simple', or 'urn'",
                options.format
            ));
        }
    };

    println!("{}", output);

    Ok(())
}
