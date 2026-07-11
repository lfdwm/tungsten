use std::{
    env,
    error::Error,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn main() -> Result<(), Box<dyn Error>> {
    println!("cargo:rerun-if-changed=shaders/fullscreen.vert");
    println!("cargo:rerun-if-changed=shaders/voxelspace.frag");
    println!("cargo:rerun-if-changed=shaders/props.vert");
    println!("cargo:rerun-if-changed=shaders/props.frag");
    println!("cargo:rerun-if-changed=shaders/water.vert");
    println!("cargo:rerun-if-changed=shaders/water.frag");
    println!("cargo:rerun-if-changed=shaders/upscale.frag");

    let out_dir = PathBuf::from(env::var("OUT_DIR")?);
    compile_shader(
        "shaders/fullscreen.vert",
        out_dir.join("fullscreen.vert.spv"),
    )?;
    compile_shader(
        "shaders/voxelspace.frag",
        out_dir.join("voxelspace.frag.spv"),
    )?;
    compile_shader("shaders/props.vert", out_dir.join("props.vert.spv"))?;
    compile_shader("shaders/props.frag", out_dir.join("props.frag.spv"))?;
    compile_shader("shaders/water.vert", out_dir.join("water.vert.spv"))?;
    compile_shader("shaders/water.frag", out_dir.join("water.frag.spv"))?;
    compile_shader("shaders/upscale.frag", out_dir.join("upscale.frag.spv"))?;

    Ok(())
}

fn compile_shader(input: impl AsRef<Path>, output: impl AsRef<Path>) -> Result<(), Box<dyn Error>> {
    let input = input.as_ref();
    let output = output.as_ref();

    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }

    let status = Command::new("glslc")
        .arg(input)
        .arg("-o")
        .arg(output)
        .status()
        .map_err(|error| format!("failed to run glslc for {}: {error}", input.display()))?;

    if !status.success() {
        return Err(format!("glslc failed while compiling {}", input.display()).into());
    }

    Ok(())
}
