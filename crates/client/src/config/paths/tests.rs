use super::*;

#[test]
fn explicit_directories_are_absolute_and_do_not_relocate_to_the_working_directory() {
    let directory = tempfile::tempdir().expect("directory fixture");
    let fallback = directory.path().join("fallback");
    let explicit = directory.path().join("explicit");
    assert_eq!(
        absolute_override(
            Some(explicit.clone().into_os_string()),
            Some(fallback.clone())
        ),
        Some(explicit)
    );
    for invalid in ["", "relative"] {
        assert_eq!(
            absolute_override(Some(invalid.into()), Some(fallback.clone())),
            Some(fallback.clone())
        );
        assert_eq!(absolute_override(Some(invalid.into()), None), None);
    }
}
