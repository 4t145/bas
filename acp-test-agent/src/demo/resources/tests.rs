use super::*;

#[test]
fn sample_link_points_to_real_content_until_the_connection_resources_are_dropped() {
    let resources = Resources::default();
    let uri = resources.sample_uri().expect("sample URI");
    let path = url::Url::parse(&uri)
        .expect("URL")
        .to_file_path()
        .expect("local path");
    assert_eq!(
        std::fs::read_to_string(&path).expect("sample file"),
        crate::demo::SAMPLE_RESOURCE
    );
    assert_eq!(resources.sample_uri().expect("stable URI"), uri);
    drop(resources);
    assert!(
        !path.exists(),
        "temporary samples do not outlive the agent connection"
    );
}
