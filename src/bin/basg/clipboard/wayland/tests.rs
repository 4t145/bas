use super::*;

#[test]
fn file_uri_list_decodes_local_paths_without_treating_remote_urls_as_files() {
    assert_eq!(
        file_paths(
            "# copied files\r\nfile:///tmp/hello%20world.txt\r\nfile://localhost/tmp/two.txt\r\nfile:///tmp/hello%20world.txt\r\nhttps://example.org/image.png\r\nfile://remote/tmp/private.txt\r\n"
        ),
        [
            std::path::PathBuf::from("/tmp/hello world.txt"),
            std::path::PathBuf::from("/tmp/two.txt")
        ]
    );
}
