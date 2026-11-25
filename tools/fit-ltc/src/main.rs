mod brdf;
mod brdf_ggx;
mod fit_tab;
mod fitter;
mod ltc;
mod pack_tab;
mod parameters;
mod sphere_tab;

use clap::Parser;
use parameters::N;
use std::{path::PathBuf, time::Instant};

#[derive(Parser)]
struct Options {
    #[arg(long, default_value = "ggx")]
    brdf: String,
    #[arg(long)]
    output: PathBuf,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = Options::parse();
    if options.brdf != "ggx" {
        return Err("only --brdf ggx is supported".into());
    }
    let start = Instant::now();
    let brdf = brdf_ggx::BrdfGGX::new();
    let (matrix, terms) = fit_tab::fit_tab(&brdf, N, N, 32);
    let sphere = sphere_tab::gen_sphere_tab(N, N);
    let (layer0, layer1) = pack_tab::pack_tab(matrix, terms, sphere, N, N);
    let mut bytes = Vec::with_capacity(8 + 2 * N * N * 4 * 4);
    bytes.extend_from_slice(b"LTC1");
    bytes.extend_from_slice(&(N as u32).to_le_bytes());
    for value in layer0
        .iter()
        .chain(layer1.iter())
        .flat_map(|v| v.to_array())
    {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    if let Some(parent) = options.output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&options.output, bytes)?;
    println!(
        "Wrote {} in {:.2?}",
        options.output.display(),
        start.elapsed()
    );
    Ok(())
}
