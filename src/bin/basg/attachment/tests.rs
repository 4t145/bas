use super::*;

#[test]
fn audio_files_use_audio_content_and_require_the_audio_capability() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("voice.wav");
    fs::write(&path, b"RIFF-fixture").expect("audio");
    let attachment = Attachment::load(&path).expect("attachment");
    let acp::ContentBlock::Audio(audio) = attachment.content.as_ref() else {
        panic!("native AudioContent")
    };
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(&audio.data)
            .expect("base64"),
        b"RIFF-fixture"
    );
    assert!(audio.mime_type.starts_with("audio/"));
    assert!(
        !attachment.supported(
            &acp::PromptCapabilities::new()
                .embedded_context(true)
                .image(true)
        )
    );
    let capabilities = acp::PromptCapabilities::new().audio(true);
    assert!(attachment.supported(&capabilities));
    let content = prompt_content(
        &attachment.mention,
        std::slice::from_ref(&attachment),
        &capabilities,
    );
    assert_eq!(content, vec![attachment.content.as_ref().clone()]);
}

#[test]
fn files_and_images_become_native_acp_content_with_escaped_uris() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let text = directory.path().join("中文 example.rs");
    fs::write(&text, "fn main() { println!(\"你好\"); }").expect("text fixture");
    let attachment = Attachment::load(&text).expect("read text");
    let acp::ContentBlock::Resource(resource) = attachment.content.as_ref() else {
        panic!("embedded resource")
    };
    let acp::EmbeddedResourceResource::TextResourceContents(content) = &resource.resource else {
        panic!("text")
    };
    assert!(content.text.contains("你好"));
    assert_eq!(
        url::Url::parse(&content.uri)
            .expect("URI")
            .to_file_path()
            .expect("path"),
        text
    );
    let image = directory.path().join("image.png");
    fs::write(&image, b"\x89PNG\r\n\x1a\n").expect("image fixture");
    let attachment = Attachment::load(&image).expect("read image");
    let acp::ContentBlock::Image(content) = attachment.content.as_ref() else {
        panic!("image")
    };
    assert_eq!(content.mime_type, "image/png");
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(&content.data)
            .expect("base64"),
        b"\x89PNG\r\n\x1a\n"
    );
    assert!(!attachment.supported(&acp::PromptCapabilities::new()));
    assert!(attachment.supported(&acp::PromptCapabilities::new().image(true)));

    let binary = directory.path().join("binary.dat");
    fs::write(&binary, b"abc\0def").expect("binary fixture");
    let attachment = Attachment::load(&binary).expect("read binary");
    let acp::ContentBlock::Resource(resource) = attachment.content.as_ref() else {
        panic!("embedded binary")
    };
    let acp::EmbeddedResourceResource::BlobResourceContents(content) = &resource.resource else {
        panic!("blob")
    };
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(&content.blob)
            .expect("binary base64"),
        b"abc\0def"
    );
}

#[test]
fn rejects_directories_and_oversized_files_without_reading_them_into_memory() {
    let directory = tempfile::tempdir().expect("temporary directory");
    assert!(Attachment::load(directory.path()).is_err());
    let path = directory.path().join("oversized");
    fs::File::create(&path)
        .expect("file")
        .set_len(MAX_FILE_BYTES + 1)
        .expect("sparse file");
    assert!(Attachment::load(&path).is_err());
}
