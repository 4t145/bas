use super::*;
use gpui_kit::{ClipboardString, ImageFormat};

#[test]
fn mixed_clipboard_preserves_text_images_and_deduplicated_file_references() {
    let path = std::env::temp_dir().join("clipboard.txt");
    let item = ClipboardItem {
        entries: vec![
            ClipboardEntry::String(ClipboardString::new("中文\r\nsecond line".into())),
            ClipboardEntry::Image(Image::from_bytes(ImageFormat::Png, vec![1, 2, 3])),
            ClipboardEntry::ExternalPaths(gpui_kit::ExternalPaths(
                vec![path.clone(), path.clone()].into(),
            )),
        ],
    };
    let contents = Contents::from_item(Some(item));
    assert_eq!(contents.text.as_deref(), Some("中文\nsecond line"));
    assert_eq!(contents.images.len(), 1);
    assert_eq!(contents.paths, [path]);
}
