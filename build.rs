//! Generates the gRPC service and client from `proto/basr/v1/basr.proto`.
//!
//! `protoc-bin-vendored` supplies the protobuf compiler so building BASR needs
//! no system `protoc`, matching the project's goal of minimal external
//! dependencies.

use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let protoc = protoc_bin_vendored::protoc_bin_path()?;
    let protoc_include = protoc_bin_vendored::include_path()?;
    let out_dir = PathBuf::from(std::env::var("OUT_DIR")?);

    let mut config = tonic_prost_build::Config::new();
    config.protoc_executable(protoc);
    // Supplies the well-known .proto files bundled with the vendored compiler.
    config.protoc_arg(format!("-I{}", protoc_include.display()));

    println!("cargo:rerun-if-changed=proto/basr/v1/basr.proto");
    println!("cargo:rerun-if-changed=build.rs");

    tonic_prost_build::configure()
        .build_server(true)
        .build_client(true)
        .file_descriptor_set_path(out_dir.join("basr_descriptor.bin"))
        .compile_with_config(config, &["proto/basr/v1/basr.proto"], &["proto"])?;

    Ok(())
}
