//! Read images and text together instead of taking GPUI's first matching pasteboard format.

use super::Contents;
use gpui_kit::{Image, ImageFormat};

#[derive(Debug, thiserror::Error)]
pub(super) enum Error {
    #[error("macOS clipboard is unavailable: {0}")]
    Clipboard(#[from] arboard::Error),
    #[error("cannot encode clipboard image: {0}")]
    Image(#[from] png::EncodingError),
    #[error("clipboard image is too large")]
    TooLarge,
}

/// # Errors
/// Reports pasteboard access, oversized images or image encoding failures.
pub(super) fn read() -> Result<Contents, Error> {
    let mut clipboard = arboard::Clipboard::new()?;
    let mut contents = Contents {
        text: optional(clipboard.get_text())?.map(|text| text.replace("\r\n", "\n")),
        paths: optional(clipboard.get().file_list())?.unwrap_or_default(),
        ..Contents::default()
    };
    if contents.paths.is_empty()
        && let Some(image) = optional(clipboard.get_image())?
    {
        const MAX_DECODED_BYTES: usize = 64 * 1024 * 1024;
        if image.bytes.len() > MAX_DECODED_BYTES {
            return Err(Error::TooLarge);
        }
        let width = u32::try_from(image.width).map_err(|_| Error::TooLarge)?;
        let height = u32::try_from(image.height).map_err(|_| Error::TooLarge)?;
        let mut data = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut data, width, height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.write_header()?.write_image_data(&image.bytes)?;
        }
        contents
            .images
            .push(Image::from_bytes(ImageFormat::Png, data));
    }
    Ok(contents)
}

fn optional<T>(result: Result<T, arboard::Error>) -> Result<Option<T>, Error> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(arboard::Error::ContentNotAvailable) => Ok(None),
        Err(error) => Err(error.into()),
    }
}
