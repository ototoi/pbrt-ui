use clap::Parser;
use uuid::Uuid;

#[derive(Debug, Parser)]
#[clap(author, about = "Generate a UUID", version)]
struct Options {
    #[arg(
        short = 'f',
        long,
        default_value = "hyphenated",
        help = "Output format: hyphenated, simple, urn, or u128"
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
        "u128" => {
            let value = uuid.as_u128();
            format!(
                "0x{:08x}_{:04x}_{:04x}_{:04x}_{:012x}",
                (value >> 96) as u32,
                ((value >> 80) & 0xFFFF) as u16,
                ((value >> 64) & 0xFFFF) as u16,
                ((value >> 48) & 0xFFFF) as u16,
                (value & 0xFFFF_FFFF_FFFF) as u64
            )
        }
        _ => {
            return Err(format!(
                "Invalid format: '{}'. Must be 'hyphenated', 'simple', 'urn', or 'u128'",
                options.format
            ));
        }
    };

    println!("{}", output);

    Ok(())
}
