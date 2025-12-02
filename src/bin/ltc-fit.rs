use pbrt_ui::ltc::brdf;
use pbrt_ui::ltc::brdf_ggx::BrdfGGX;

use std::sync::Arc;

use clap::*;

#[derive(Debug, Parser)]
#[clap(author, about, version, disable_help_flag = true)]
struct Options {
    brdf: String,
}

fn create_brdf(brdf_name: &str) -> Result<Arc<dyn brdf::Brdf>, String> {
    match brdf_name {
        "ggx" => Ok(Arc::new(BrdfGGX::new())),
        _ => Err(format!("Unknown BRDF name: {}", brdf_name)),
    }
}

fn main() -> Result<(), String> {
    // Placeholder for LTC fitting logic
    let options = Options::parse();
    let brdf = create_brdf(&options.brdf)?;


    Ok(())
}