//! Emit build provenance for BAS executables.
mod build_metadata;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    build_metadata::emit()
}
