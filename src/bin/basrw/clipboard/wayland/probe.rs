//! Native data-control checks run inside the private compositor created by the test runner.

use super::*;
use wl_clipboard_rs::copy::{self, MimeSource, Options, Source};

/// # Errors
/// Reports compositor, format negotiation and clipboard content failures.
pub(crate) fn check() -> anyhow::Result<()> {
    anyhow::ensure!(
        std::env::var_os("BASRW_TEST_ISOLATED_CLIPBOARD").is_some(),
        "run tests/wayland_clipboard.py to use a private clipboard"
    );
    let write = |sources: Vec<(&str, Vec<u8>)>| -> anyhow::Result<()> {
        let mut options = Options::new();
        options.omit_additional_text_mime_types(true);
        copy::copy_multi(
            options,
            sources
                .into_iter()
                .map(|(mime, data)| MimeSource {
                    mime_type: copy::MimeType::Specific(mime.to_owned()),
                    source: Source::Bytes(data.into_boxed_slice()),
                })
                .collect(),
        )?;
        Ok(())
    };
    write(vec![(
        "text/plain",
        "中文\r\nplain-only".as_bytes().to_vec(),
    )])?;
    let contents = read()?;
    anyhow::ensure!(
        contents.text.as_deref() == Some("中文\nplain-only"),
        "plain-only MIME and CRLF normalization"
    );
    anyhow::ensure!(!contents.has_attachments(), "text is not an attachment");

    use base64::Engine as _;
    let image = base64::engine::general_purpose::STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLttAAAAABJRU5ErkJggg==")?;
    write(vec![
        ("text/plain;charset=utf-8", b"image description".to_vec()),
        ("image/png", image.clone()),
    ])?;
    let contents = read()?;
    anyhow::ensure!(
        contents.text.as_deref() == Some("image description"),
        "mixed clipboard text survives"
    );
    anyhow::ensure!(
        contents.images.len() == 1 && contents.images[0].bytes() == image.as_slice(),
        "mixed clipboard image survives"
    );

    write(vec![(
        "text/uri-list",
        b"# files\r\nfile:///tmp/a%20b.txt\r\nfile://remote/tmp/reject.txt\r\n".to_vec(),
    )])?;
    let contents = read()?;
    anyhow::ensure!(
        contents.paths == [std::path::PathBuf::from("/tmp/a b.txt")],
        "URI list retains only local files"
    );
    println!("Wayland clipboard: plain text, image + text, and file URI checks passed");
    Ok(())
}
