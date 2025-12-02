use pbrt_ui::ltc::brdf::Brdf;
use pbrt_ui::ltc::brdf_beckmann::BrdfBeckmann;
use pbrt_ui::ltc::brdf_disneydiffuse::BrdfDisneyDiffuse;
use pbrt_ui::ltc::brdf_ggx::BrdfGGX;
use pbrt_ui::ltc::fit_tab::fit_tab;
use pbrt_ui::ltc::pack_tab::pack_tab;
use pbrt_ui::ltc::parameters::N;
use pbrt_ui::ltc::sphere_tab::gen_sphere_tab;

use std::path::PathBuf;
use std::sync::Arc;

use clap::*;
use image::{Rgba, ImageBuffer};

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
    let _tab_sphere = gen_sphere_tab(N);
    println!("Sphere table generated successfully!");

    println!();
    println!("Packing tables...");
    let (tex1, tex2) = pack_tab(tab, tab_mag_fresnel);
    println!("Tables packed successfully!");

    if let Some(output_path) = options.output {
        println!();
        println!("Saving textures to: {}", output_path.display());
        
        // Create output directory if it doesn't exist
        std::fs::create_dir_all(&output_path)
            .map_err(|e| format!("Failed to create output directory: {}", e))?;

        // Save tex1
        let tex1_path = output_path.join("tex1.exr");
        let tex1_img: ImageBuffer<Rgba<f32>, Vec<f32>> = ImageBuffer::from_fn(N as u32, N as u32, |x, y| {
            let idx = (x as usize) + (y as usize) * N;
            let v = tex1[idx];
            Rgba([v.x, v.y, v.z, v.w])
        });
        tex1_img.save(&tex1_path)
            .map_err(|e| format!("Failed to save tex1.exr: {}", e))?;
        println!("Saved: {}", tex1_path.display());

        // Save tex2
        let tex2_path = output_path.join("tex2.exr");
        let tex2_img: ImageBuffer<Rgba<f32>, Vec<f32>> = ImageBuffer::from_fn(N as u32, N as u32, |x, y| {
            let idx = (x as usize) + (y as usize) * N;
            let v = tex2[idx];
            Rgba([v.x, v.y, v.z, v.w])
        });
        tex2_img.save(&tex2_path)
            .map_err(|e| format!("Failed to save tex2.exr: {}", e))?;
        println!("Saved: {}", tex2_path.display());
    }

    println!();
    println!("LTC fitting completed successfully!");
    
    Ok(())
}
