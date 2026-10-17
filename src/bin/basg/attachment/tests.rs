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

#[test]
fn nix_text_supports_agents_without_embedded_context_but_binary_still_requires_it() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("flake.nix");
    let source = "{ pkgs, ... }: { programs.git.enable = true; }\n";
    fs::write(&path, source).expect("Nix fixture");
    let text = Attachment::load(&path).expect("Nix text");
    let plain = acp::PromptCapabilities::new();
    assert!(text.supported(&plain));
    let fallback = prompt_content(&text.mention, std::slice::from_ref(&text), &plain);
    let acp::ContentBlock::Text(block) = &fallback[0] else {
        panic!("plain-text fallback")
    };
    assert!(block.text.contains("flake.nix"));
    assert!(block.text.contains(&format!("```nix\n{source}\n```")));
    let rich = prompt_content(
        &text.mention,
        std::slice::from_ref(&text),
        &plain.clone().embedded_context(true),
    );
    let acp::ContentBlock::Resource(resource) = &rich[0] else {
        panic!("native embedded resource")
    };
    let acp::EmbeddedResourceResource::TextResourceContents(block) = &resource.resource else {
        panic!("text resource")
    };
    assert_eq!(block.mime_type.as_deref(), Some("text/x-nix"));
    assert_eq!(block.text, source);
    let path = directory.path().join("binary.dat");
    fs::write(&path, b"\0\xff\x00").expect("binary fixture");
    let binary = Attachment::load(&path).expect("binary");
    assert!(!binary.supported(&plain));
    assert!(binary.supported(&plain.embedded_context(true)));
}

#[test]
fn file_code_preserves_comments_and_nested_fences_as_literal_text() {
    let source = "# 全机 SSH 公钥清单\n```\n# still file contents\n```";
    assert_eq!(
        file_code(source, "file:///keys.nix"),
        format!("````nix\n{source}\n````")
    );
}
