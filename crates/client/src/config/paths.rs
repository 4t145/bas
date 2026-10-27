//! Shared user directories for GUI settings, desktop state and daemon storage.

use std::{ffi::OsString, path::PathBuf};

pub fn config_root() -> Option<PathBuf> {
    absolute_override(std::env::var_os("XDG_CONFIG_HOME"), dirs::config_dir())
}

pub fn cache_root() -> Option<PathBuf> {
    absolute_override(std::env::var_os("XDG_CACHE_HOME"), dirs::cache_dir())
}

pub fn state_root() -> Option<PathBuf> {
    absolute_override(
        std::env::var_os("XDG_STATE_HOME"),
        dirs::state_dir().or_else(dirs::data_local_dir),
    )
}

fn absolute_override(value: Option<OsString>, fallback: Option<PathBuf>) -> Option<PathBuf> {
    value
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or(fallback)
}

#[cfg(test)]
mod tests;
