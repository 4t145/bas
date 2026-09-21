//! Read snapshots atomically from the renderer's perspective; reject failed reloads.

use super::{
    preferences::{self, Effective, Preferences, ThemeMode, ThemeSource},
    source::{self, Files, Sources},
};
use anyhow::{Context as _, Result};
use std::path::PathBuf;

#[cfg(test)]
mod tests;

#[derive(Clone)]
pub(crate) struct Document {
    pub preferences: Preferences,
    original: Option<String>,
}

#[derive(Debug, thiserror::Error)]
enum SaveError {
    #[error("配置已被外部修改；请关闭并重新打开设置后再保存")]
    Conflict,
    #[error("配置是符号链接（可能由 Nix 管理）；请修改其声明来源，而不是在此覆盖")]
    Symlink,
}

#[derive(Clone)]
pub(super) struct Loader {
    pub appearance_file: PathBuf,
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
    pub fn new(appearance_file: Option<PathBuf>, theme_file: Option<PathBuf>) -> Self {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config")
            });
        Self {
            appearance_file: appearance_file.unwrap_or_else(|| config.join("basr/appearance.json")),
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
        let original = source::read(&self.appearance_file)?;
        let preferences = original
            .as_deref()
            .map(Preferences::from_json)
            .transpose()
            .with_context(|| format!("appearance preferences {}", self.appearance_file.display()))?
            .unwrap_or_default();
        Ok(Document {
            preferences,
            original,
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
        use std::io::Write as _;
        let effective = self.snapshot(preferences.clone())?.resolve(dark)?;
        let text = serde_json::to_string_pretty(&preferences)? + "\n";
        self.check_original(document)?;
        let parent = self
            .appearance_file
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(std::path::Path::new("."));
        std::fs::create_dir_all(parent).context("无法创建配置目录")?;
        let mut temporary =
            tempfile::NamedTempFile::new_in(parent).context("无法创建临时配置文件")?;
        temporary
            .write_all(text.as_bytes())
            .context("无法写入配置")?;
        temporary.as_file().sync_all().context("无法同步配置")?;
        self.check_original(document)?;
        temporary
            .persist(&self.appearance_file)
            .context("无法原子替换配置")?;
        document.preferences = preferences;
        document.original = Some(text);
        Ok(effective)
    }

    /// # Errors
    /// Rejects conflicting edits, symbolic links, or an unreadable target.
    fn check_original(&self, document: &Document) -> Result<()> {
        match std::fs::symlink_metadata(&self.appearance_file) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(SaveError::Symlink.into());
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        if source::read(&self.appearance_file)? != document.original {
            return Err(SaveError::Conflict.into());
        }
        Ok(())
    }

    fn theme_path(&self, preferences: &Preferences) -> Option<PathBuf> {
        self.theme_file.clone().or_else(|| {
            preferences.theme_file.as_ref().map(|path| {
                if path.is_absolute() {
                    path.clone()
                } else {
                    self.appearance_file
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
