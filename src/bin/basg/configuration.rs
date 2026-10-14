//! The selected configuration store is shared by all frontend features.

use std::path::PathBuf;

use bas::config::Store;
use gpui_kit::{App, Global};

struct Configuration(Store);
impl Global for Configuration {}

pub(crate) fn init(settings_file: Option<PathBuf>, cx: &mut App) {
    cx.set_global(Configuration(Store::new(settings_file)));
}

pub(crate) fn store(cx: &App) -> Store {
    cx.global::<Configuration>().0.clone()
}

#[cfg(all(test, feature = "gui-tests"))]
mod tests;
