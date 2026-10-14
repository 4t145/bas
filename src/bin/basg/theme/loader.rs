//! Read snapshots atomically from the renderer's perspective; reject failed reloads.

use super::{
    preferences::{self, Effective, Preferences, ThemeMode, ThemeSource},
    source::{self, Files, Sources},
};
use anyhow::{Context as _, Result};
use bas::config::{Document as SettingsDocument, Store};
use std::path::PathBuf;

#[cfg(test)]
mod tests;

#[derive(Clone)]
pub(crate) struct Document {
    pub preferences: Preferences,
    settings: SettingsDocument,
}

#[derive(Clone)]
pub(super) struct Loader {
    pub store: Store,
    pub theme_file: Option<PathBuf>,
    #[cfg(test)]
    pub test_imports: Option<Files>,
}

#[derive(Clone, PartialEq)]
pub(super) struct Snapshot {
    preferences: Preferences,
    files: Files,
    source_description: String,
}

#[derive(Default)]
pub(super) struct Reload {
    previous: Option<(Snapshot, bool)>,
    generation: u64,
}

impl Reload {
    pub fn invalidate_on_save(&mut self, generation: u64) {
        if self.generation != generation {
            self.previous = None;
            self.generation = generation;
        }
    }

    /// # Errors
    /// Invalid candidates leave the last successfully resolved snapshot unchanged.
    pub fn update(&mut self, snapshot: Snapshot, dark: bool) -> Result<Option<Effective>> {
        if self
            .previous
            .as_ref()
            .is_some_and(|(previous, mode)| previous == &snapshot && *mode == dark)
        {
            return Ok(None);
        }
        let effective = snapshot.resolve(dark)?;
        self.previous = Some((snapshot, dark));
        Ok(Some(effective))
    }
}

impl Loader {
    pub fn new(store: Store, theme_file: Option<PathBuf>) -> Self {
        Self {
            store,
            theme_file,
            #[cfg(test)]
            test_imports: None,
        }
    }

    /// # Errors
    /// Reports read, parse, or validation errors with the affected file path.
    pub fn read(&self) -> Result<Snapshot> {
        self.snapshot(self.document()?.preferences)
    }

    /// # Errors
    /// Reports unreadable or malformed preferences without replacing them.
    pub fn document(&self) -> Result<Document> {
        let settings = self.store.load()?;
        Ok(Document {
            preferences: settings.settings.appearance.clone(),
            settings,
        })
    }

    /// # Errors
    /// Rejects invalid preferences or unreadable desktop imports.
    fn snapshot(&self, preferences: Preferences) -> Result<Snapshot> {
        preferences.validate()?;
        let sources = Sources::discover(self.theme_path(&preferences));
        let files = match preferences.theme_source {
            ThemeSource::Desktop => {
                #[cfg(test)]
                if let Some(files) = &self.test_imports {
                    return Ok(Snapshot {
                        preferences,
                        files: files.clone(),
                        source_description: "isolated test imports".into(),
                    });
                }
                sources.read()?
            }
            ThemeSource::Builtin => Files::default(),
        };
        Ok(Snapshot {
            preferences,
            files,
            source_description: sources.description(),
        })
    }

    /// # Errors
    /// Rejects invalid candidates, external edits, symlinks and filesystem errors.
    pub fn save(
        &self,
        document: &mut Document,
        preferences: Preferences,
        dark: bool,
    ) -> Result<Effective> {
        let effective = self.snapshot(preferences.clone())?.resolve(dark)?;
        self.store
            .save_appearance(&mut document.settings, preferences.clone())?;
        document.preferences = preferences;
        Ok(effective)
    }

    fn theme_path(&self, preferences: &Preferences) -> Option<PathBuf> {
        self.theme_file.clone().or_else(|| {
            preferences.theme_file.as_ref().map(|path| {
                if path.is_absolute() {
                    path.clone()
                } else {
                    self.store
                        .path()
                        .parent()
                        .unwrap_or(std::path::Path::new("."))
                        .join(path)
                }
            })
        })
    }
}

impl Snapshot {
    /// # Errors
    /// Rejects malformed imports or invalid effective sizes without changing live state.
    pub fn resolve(&self, system_dark: bool) -> Result<Effective> {
        let mode = match self.preferences.theme_mode {
            ThemeMode::Desktop => None,
            ThemeMode::System => Some(system_dark),
            ThemeMode::Dark => Some(true),
            ThemeMode::Light => Some(false),
        };
        let dark = mode.unwrap_or(system_dark);
        let desktop = match self.preferences.theme_source {
            ThemeSource::Desktop => match mode {
                None => source::resolve(&self.files, dark),
                Some(_) => source::resolve_mode(&self.files, dark, mode),
            }
            .with_context(|| self.source_description.clone())?,
            ThemeSource::Builtin => source::fallback(dark),
        };
        preferences::resolve(&self.preferences, desktop)
    }
}
