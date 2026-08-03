use clap::Parser as ClapParser;
use dx9_compiler::codegen::Codegen;
use dx9_compiler::parser::Parser;
use dx9_compiler::preprocess::{PreprocessOptions, preprocess};
use std::fs;
use std::path::PathBuf;

#[derive(ClapParser, Debug)]
#[command(
    name = "dx9-compiler",
    version = "0.1.0",
    about = "DX9 Shader Compiler in Rust"
)]
struct Args {
    /// Input HLSL shader file (.fxc)
    input: PathBuf,

    /// Output compiled shader file
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Compile as pixel shader (default is vertex shader)
    #[arg(short, long)]
    pixel: bool,

    /// Extra -DNAME[=VALUE] macros
    #[arg(short = 'D', long = "define", value_name = "NAME[=VALUE]")]
    defines: Vec<String>,
}

fn main() {
    let args = Args::parse();

    let source = match fs::read_to_string(&args.input) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error reading input file: {}", e);
            std::process::exit(1);
        }
    };

    let is_pixel_shader = args.pixel || args.input.to_string_lossy().contains("_ps");

    let mut options = PreprocessOptions::default();
    if let Some(parent) = args.input.parent() {
        options.include_dirs.push(parent.to_path_buf());
    }
    if is_pixel_shader {
        options
            .defines
            .insert("SHADER_MODEL_PS_3_0".into(), "1".into());
    } else {
        options
            .defines
            .insert("SHADER_MODEL_VS_3_0".into(), "1".into());
    }
    for def in &args.defines {
        if let Some((name, value)) = def.split_once('=') {
            options.defines.insert(name.to_string(), value.to_string());
        } else {
            options.defines.insert(def.clone(), "1".into());
        }
    }

    let source = match preprocess(&source, &options) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Preprocess error: {e}");
            std::process::exit(1);
        }
    };

    let filename_str = args.input.to_string_lossy();
    let mut parser = Parser::new(&source, &filename_str);
    let ast = parser.parse();

    let codegen = Codegen::new();
    let bytecode = codegen.compile(&ast, is_pixel_shader);

    let mut binary_data = Vec::with_capacity(bytecode.len() * 4);
    for word in bytecode {
        binary_data.extend_from_slice(&word.to_le_bytes());
    }

    let output_path = args.output.unwrap_or_else(|| {
        let mut path = args.input.clone();
        path.set_extension("bin");
        path
    });

    if let Err(e) = fs::write(&output_path, &binary_data) {
        eprintln!("Error writing output file: {}", e);
        std::process::exit(1);
    }

    println!(
        "Successfully compiled {} to {} ({} bytes)",
        args.input.display(),
        output_path.display(),
        binary_data.len()
    );
}
