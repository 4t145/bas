use basr::model::SessionMeta;
use std::path::{Path, PathBuf};

#[cfg(test)]
mod tests;

pub(super) struct Group<'a> {
    pub directory: Option<PathBuf>,
    pub threads: Vec<&'a SessionMeta>,
}

pub(super) fn group_threads<'a>(
    sessions: &'a [SessionMeta],
    directories: &[PathBuf],
    query: &str,
) -> Vec<Group<'a>> {
    let mut groups: Vec<Group<'a>> = Vec::new();
    for session in sessions
        .iter()
        .filter(|session| super::matches_query(session, query))
    {
        let directory = session.cwd.as_deref().map(normalize);
        if let Some(group) = groups.iter_mut().find(|group| group.directory == directory) {
            group.threads.push(session);
        } else {
            groups.push(Group {
                directory,
                threads: vec![session],
            });
        }
    }
    for directory in directories {
        let directory = normalize(directory);
        if (query.is_empty() || directory.to_string_lossy().to_lowercase().contains(query))
            && !groups
                .iter()
                .any(|group| group.directory.as_ref() == Some(&directory))
        {
            groups.push(Group {
                directory: Some(directory),
                threads: Vec::new(),
            });
        }
    }
    groups
}

pub(super) fn name(directory: Option<&Path>) -> String {
    directory
        .map(|path| {
            path.file_name()
                .unwrap_or(path.as_os_str())
                .to_string_lossy()
                .into_owned()
        })
        .unwrap_or_else(|| "默认 workspace".into())
}

pub(super) fn path_label(directory: Option<&Path>) -> String {
    directory
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| "basrd 默认目录".into())
}

pub(super) fn normalize(path: &Path) -> PathBuf {
    // These are daemon-side paths. Local filesystem canonicalization would be incorrect for remote basrd.
    path.components().collect()
}
