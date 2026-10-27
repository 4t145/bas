//! Generate protocol messages and client stubs from the canonical BAS schema.
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?);
    let schema = root.join("proto/bas/v1/bas.proto");
    let protoc = protoc_bin_vendored::protoc_bin_path()?;
    let mut config = tonic_prost_build::Config::new();
    config.protoc_executable(protoc);
    config.protoc_arg(format!(
        "-I{}",
        protoc_bin_vendored::include_path()?.display()
    ));
    let out = PathBuf::from(std::env::var("OUT_DIR")?);
    println!("cargo:rerun-if-changed={}", schema.display());
    tonic_prost_build::configure()
        .build_server(cfg!(feature = "server"))
        .build_client(true)
        .file_descriptor_set_path(out.join("bas_descriptor.bin"))
        .compile_with_config(config, &[schema], &[root.join("proto")])?;
    Ok(())
}
