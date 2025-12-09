use pbrt_ui::ltc::brdf::Brdf;
use pbrt_ui::ltc::brdf_beckmann::BrdfBeckmann;
use pbrt_ui::ltc::brdf_disney_diffuse::BrdfDisneyDiffuse;
use pbrt_ui::ltc::brdf_ggx::BrdfGGX;
use pbrt_ui::ltc::brdf_microfacet_reflection::BrdfMicrofacetReflection;
use pbrt_ui::ltc::brdf_oren_nayar::BrdfOrenNayar;
use pbrt_ui::ltc::export::{write_exr, write_multiple_ltc_arrays};
use pbrt_ui::ltc::fit_tab::fit_tab;
use pbrt_ui::ltc::pack_tab::pack_tab;
use pbrt_ui::ltc::parameters::N;
use pbrt_ui::ltc::sphere_tab::gen_sphere_tab;

use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use convert_case::{Case, Casing};

#[derive(Debug, Parser)]
#[clap(author, about, version, disable_help_flag = true)]
struct Options {
    #[arg(short = 'b', long, default_value = "ggx")]
    brdf: String,

    #[arg(short = 'o', long, help = "Output directory path for texture files")]
    output: Option<PathBuf>,

    #[arg(
        short = 'f',
        long,
        value_delimiter = ',',
        help = "Output format: exr or code (can specify multiple times or comma-separated)"
    )]
    output_format: Vec<String>,

    #[arg(short = 'w', long, default_value_t = N, help = "Table width (default: 64)")]
    width: usize,

    #[arg(short = 'h', long = "height", default_value_t = N, help = "Table height (default: 64)")]
    table_height: usize,
}

fn create_brdf(brdf_name: &str) -> Result<Arc<dyn Brdf>, String> {
    let brdf_name = brdf_name.to_lowercase();
    match brdf_name.as_str() {
        "ggx" => Ok(Arc::new(BrdfGGX::new())),
        "beckmann" => Ok(Arc::new(BrdfBeckmann::new())),
        "disneydiffuse" | "disney_diffuse" | "disney-diffuse" => {
            Ok(Arc::new(BrdfDisneyDiffuse::new()))
        }
        "orennayar" | "oren_nayar" | "oren-nayar" => Ok(Arc::new(BrdfOrenNayar::new())),
        "microfacet_reflection" | "microfacetreflection" | "microfacet-reflection" => {
            Ok(Arc::new(BrdfMicrofacetReflection::new()))
        }
        _ => Err(format!("Unknown BRDF name: {}", brdf_name)),
    }
}

fn main() -> Result<(), String> {
    let options = Options::parse();
    let brdf_name = options.brdf.to_string();
    let brdf_name = brdf_name.to_case(Case::Snake);
    let brdf_name = brdf_name.to_lowercase();
    let brdf = create_brdf(&brdf_name)?;

    // If no format specified, default to all formats
    let output_formats: Vec<String> = if options.output_format.is_empty() {
        vec!["exr".to_string(), "code".to_string()]
    } else {
        options.output_format
    };

    // Validate output formats
    for format in &output_formats {
        if format != "exr" && format != "code" {
            return Err(format!(
                "Invalid output format: '{}'. Must be 'exr' or 'code'",
                format
            ));
        }
    }

    println!("Fitting LTC table for BRDF: {}", brdf_name);
    println!("Table size: {}x{}", options.width, options.table_height);
    println!();

    let (tab, tab_mag_fresnel) = fit_tab(brdf.as_ref(), options.width, options.table_height);

    println!();
    println!("Generating sphere table...");
    let tab_sphere = gen_sphere_tab(options.width, options.table_height);
    println!("Sphere table generated successfully!");

    println!();
    println!("Packing tables...");
    let (tex1, tex2) = pack_tab(
        tab,
        tab_mag_fresnel,
        tab_sphere,
        options.width,
        options.table_height,
    );
    println!("Tables packed successfully!");

    if let Some(output_path) = options.output {
        println!();
        println!("Saving textures to: {}", output_path.display());

        // Create output directory if it doesn't exist
        std::fs::create_dir_all(&output_path)
            .map_err(|e| format!("Failed to create output directory: {}", e))?;

        // Export each specified format
        for format in &output_formats {
            match format.as_str() {
                "exr" => {
                    // Save tex1
                    let tex1_path = output_path.join(format!("ltc_{}_tex1.exr", brdf_name));
                    write_exr(&tex1_path, &tex1, options.width, options.table_height)?;
                    println!("Saved: {}", tex1_path.display());

                    // Save tex2
                    let tex2_path = output_path.join(format!("ltc_{}_tex2.exr", brdf_name));
                    write_exr(&tex2_path, &tex2, options.width, options.table_height)?;
                    println!("Saved: {}", tex2_path.display());
                }
                "code" => {
                    // Save as Rust code
                    let code_path = output_path.join(format!("ltc_{}.rs", brdf_name));
                    write_multiple_ltc_arrays(
                        &code_path,
                        &brdf_name,
                        &tex1,
                        &tex2,
                        options.width,
                        options.table_height,
                    )?;
                    println!("Saved: {}", code_path.display());
                }
                _ => unreachable!("Format validation should prevent this"),
            }
        }
    }

    println!();
    println!("LTC fitting completed successfully!");

    Ok(())
}
