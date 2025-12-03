use pbrt_ui::ltc::brdf::Brdf;
use pbrt_ui::ltc::brdf_beckmann::BrdfBeckmann;
use pbrt_ui::ltc::brdf_disneydiffuse::BrdfDisneyDiffuse;
use pbrt_ui::ltc::brdf_ggx::BrdfGGX;
use pbrt_ui::ltc::export::write_exr;
use pbrt_ui::ltc::fit_tab::fit_tab;
use pbrt_ui::ltc::pack_tab::pack_tab;
use pbrt_ui::ltc::parameters::N;
use pbrt_ui::ltc::sphere_tab::gen_sphere_tab;

use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;

#[derive(Debug, Parser)]
#[clap(author, about, version, disable_help_flag = true)]
struct Options {
    #[arg(default_value = "ggx")]
    brdf: String,
    
    #[arg(short, long, help = "Output directory path for texture files")]
    output: Option<PathBuf>,
}

fn create_brdf(brdf_name: &str) -> Result<Arc<dyn Brdf>, String> {
    match brdf_name {
        "ggx" => Ok(Arc::new(BrdfGGX::new())),
        "beckmann" => Ok(Arc::new(BrdfBeckmann::new())),
        "disneydiffuse" => Ok(Arc::new(BrdfDisneyDiffuse::new())),
        _ => Err(format!("Unknown BRDF name: {}", brdf_name)),
    }
}

fn main() -> Result<(), String> {
    let options = Options::parse();
    let brdf = create_brdf(&options.brdf)?;

    println!("Fitting LTC table for BRDF: {}", options.brdf);
    println!("Table size: {}x{}", N, N);
    println!();

    let (tab, tab_mag_fresnel) = fit_tab(brdf.as_ref());

    println!();
    println!("Generating sphere table...");
    let tab_sphere = gen_sphere_tab(N);
    println!("Sphere table generated successfully!");

    println!();
    println!("Packing tables...");
    let (tex1, tex2) = pack_tab(tab, tab_mag_fresnel, tab_sphere);
    println!("Tables packed successfully!");

    if let Some(output_path) = options.output {
        println!();
        println!("Saving textures to: {}", output_path.display());
        
        // Create output directory if it doesn't exist
        std::fs::create_dir_all(&output_path)
            .map_err(|e| format!("Failed to create output directory: {}", e))?;

        // Save tex1
        let tex1_path = output_path.join("tex1.exr");
        write_exr(&tex1_path, &tex1)?;
        println!("Saved: {}", tex1_path.display());

        // Save tex2
        let tex2_path = output_path.join("tex2.exr");
        write_exr(&tex2_path, &tex2)?;
        println!("Saved: {}", tex2_path.display());
    }

    println!();
    println!("LTC fitting completed successfully!");
    
    Ok(())
}
