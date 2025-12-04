use pbrt_ui::ltc::brdf::Brdf;
use pbrt_ui::ltc::brdf_beckmann::BrdfBeckmann;
use pbrt_ui::ltc::brdf_disneydiffuse::BrdfDisneyDiffuse;
use pbrt_ui::ltc::brdf_ggx::BrdfGGX;
use pbrt_ui::ltc::brdf_oren_nayar::BrdfOrenNayar;
use pbrt_ui::ltc::export::{write_exr, write_multiple_ltc_arrays};
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

    #[arg(
        short = 'f',
        long,
        default_value = "exr",
        help = "Output format: exr or code"
    )]
    output_format: String,

    #[arg(short = 'w', long, default_value_t = N, help = "Table width (default: 64)")]
    width: usize,

    #[arg(short = 'h', long = "height", default_value_t = N, help = "Table height (default: 64)")]
    table_height: usize,
}

fn create_brdf(brdf_name: &str) -> Result<Arc<dyn Brdf>, String> {
    match brdf_name {
        "ggx" => Ok(Arc::new(BrdfGGX::new())),
        "beckmann" => Ok(Arc::new(BrdfBeckmann::new())),
        "disneydiffuse" => Ok(Arc::new(BrdfDisneyDiffuse::new())),
        "orennayar" => Ok(Arc::new(BrdfOrenNayar::new())),
        _ => Err(format!("Unknown BRDF name: {}", brdf_name)),
    }
}

fn main() -> Result<(), String> {
    let options = Options::parse();
    let brdf = create_brdf(&options.brdf)?;

    // Validate output format
    if options.output_format != "exr" && options.output_format != "code" {
        return Err(format!(
            "Invalid output format: '{}'. Must be 'exr' or 'code'",
            options.output_format
        ));
    }

    println!("Fitting LTC table for BRDF: {}", options.brdf);
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

        match options.output_format.as_str() {
            "exr" => {
                // Save tex1
                let tex1_path = output_path.join(format!("ltc_{}_tex1.exr", options.brdf));
                write_exr(&tex1_path, &tex1, options.width, options.table_height)?;
                println!("Saved: {}", tex1_path.display());

                // Save tex2
                let tex2_path = output_path.join(format!("ltc_{}_tex2.exr", options.brdf));
                write_exr(&tex2_path, &tex2, options.width, options.table_height)?;
                println!("Saved: {}", tex2_path.display());
            }
            "code" => {
                // Save as Rust code
                let code_path = output_path.join(format!("ltc_{}.rs", options.brdf));
                write_multiple_ltc_arrays(
                    &code_path,
                    &options.brdf,
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

    println!();
    println!("LTC fitting completed successfully!");

    Ok(())
}
