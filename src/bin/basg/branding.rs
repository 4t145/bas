use gds_uim::el;
use gpui_kit::{ImageSource, Img, Resource, Styled, img, px};

use crate::assets::DIMENSIONAL;

const LOGO_SIZE: f32 = 32.;

/// Full-color artwork for large displays, independent of the theme's outline ink.
pub(crate) fn dimensional_logo() -> Img {
    let source = ImageSource::Resource(Resource::Embedded(DIMENSIONAL.into()));
    el! { #img(source) { size: LOGO_SIZE.px; flex_shrink_0; } }
}

#[cfg(all(test, feature = "gui-tests"))]
mod tests;
