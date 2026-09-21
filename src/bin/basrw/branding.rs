use gpui_kit::{App, ImageSource, Img, Resource, Styled, component::Theme, img, px};

use crate::assets::{OUTLINE, OUTLINE_LIGHT};

const LOGO_SIZE: f32 = 32.;

/// The original v4 outline; callers set its display size without changing the artwork.
pub(crate) fn logo(cx: &App) -> Img {
    // `svg()` produces a tinted alpha mask, losing the supplied SVG's colors.
    img(ImageSource::Resource(Resource::Embedded(
        logo_path(cx).into(),
    )))
    .size(px(LOGO_SIZE))
    .flex_shrink_0()
}

fn logo_path(cx: &App) -> &'static str {
    if Theme::global(cx).is_dark() {
        OUTLINE_LIGHT
    } else {
        OUTLINE
    }
}

#[cfg(all(test, feature = "gui-tests"))]
mod tests;
