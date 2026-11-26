//! The list of recently opened projects shown on the home screen.
//!
//! Stored as JSON in the app's configuration folder, most recent first. The list
//! only holds paths: a project moved or deleted since is shown as missing and can
//! be removed from the list, never searched for.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::project_view::project_name_of;

/// File name of the list in the configuration folder.
const RECENT_PROJECTS_FILE_NAME: &str = "recent-projects.json";
/// The home screen shows this many projects at most.
const MAXIMUM_RECENT_PROJECTS: usize = 24;

/// One entry as stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredRecentProject {
    file_path: PathBuf,
    last_opened_in_milliseconds: u64,
}

/// One entry as the home screen shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentProject {
    /// The `.kadr` file.
    pub file_path: String,
    /// The project name (file name without extension).
    pub name: String,
    /// When it was last opened, in milliseconds since 1970 (UTC).
    pub last_opened_in_milliseconds: u64,
    /// Whether the file still exists.
    pub exists: bool,
}

/// The recent projects, most recent first. An unreadable list counts as empty.
pub fn load_recent_projects(configuration_folder: &Path) -> Vec<RecentProject> {
    read_stored_list(configuration_folder)
        .into_iter()
        .map(|stored| RecentProject {
            name: project_name_of(&stored.file_path),
            exists: stored.file_path.exists(),
            file_path: stored.file_path.display().to_string(),
            last_opened_in_milliseconds: stored.last_opened_in_milliseconds,
        })
        .collect()
}

/// Puts `file_path` at the top of the list (now). If `previous_path` is given
/// (the project was renamed or saved elsewhere), that entry is replaced.
///
/// # Errors
///
/// When the list cannot be written.
pub fn remember_recent_project(
    configuration_folder: &Path,
    file_path: &Path,
    previous_path: Option<&Path>,
) -> std::io::Result<()> {
    let mut list = read_stored_list(configuration_folder);
    list.retain(|stored| {
        stored.file_path != file_path && Some(stored.file_path.as_path()) != previous_path
    });
    list.insert(
        0,
        StoredRecentProject {
            file_path: file_path.to_path_buf(),
            last_opened_in_milliseconds: now_in_milliseconds(),
        },
    );
    list.truncate(MAXIMUM_RECENT_PROJECTS);
    write_stored_list(configuration_folder, &list)
}

/// Removes `file_path` from the list (the file itself is not touched).
///
/// # Errors
///
/// When the list cannot be written.
pub fn forget_recent_project(configuration_folder: &Path, file_path: &Path) -> std::io::Result<()> {
    let mut list = read_stored_list(configuration_folder);
    list.retain(|stored| stored.file_path != file_path);
    write_stored_list(configuration_folder, &list)
}

fn read_stored_list(configuration_folder: &Path) -> Vec<StoredRecentProject> {
    fs::read_to_string(configuration_folder.join(RECENT_PROJECTS_FILE_NAME))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn write_stored_list(
    configuration_folder: &Path,
    list: &[StoredRecentProject],
) -> std::io::Result<()> {
    fs::create_dir_all(configuration_folder)?;
    let text = serde_json::to_string_pretty(list).map_err(std::io::Error::other)?;
    fs::write(configuration_folder.join(RECENT_PROJECTS_FILE_NAME), text)
}

fn now_in_milliseconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_opened_project_comes_first_and_renames_replace_entries() {
        let folder = tempfile::tempdir().unwrap();
        let first = folder.path().join("First.kadr");
        let second = folder.path().join("Second.kadr");
        fs::write(&first, "{}").unwrap();

        remember_recent_project(folder.path(), &first, None).unwrap();
        remember_recent_project(folder.path(), &second, None).unwrap();
        remember_recent_project(folder.path(), &first, None).unwrap();
        let list = load_recent_projects(folder.path());
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "First");
        assert!(list[0].exists);
        assert!(!list[1].exists);

        let renamed = folder.path().join("Renamed.kadr");
        remember_recent_project(folder.path(), &renamed, Some(&first)).unwrap();
        let names: Vec<String> = load_recent_projects(folder.path())
            .into_iter()
            .map(|recent| recent.name)
            .collect();
        assert_eq!(names, vec!["Renamed", "Second"]);

        forget_recent_project(folder.path(), &second).unwrap();
        assert_eq!(load_recent_projects(folder.path()).len(), 1);
    }

    #[test]
    fn a_damaged_list_counts_as_empty() {
        let folder = tempfile::tempdir().unwrap();
        fs::write(folder.path().join(RECENT_PROJECTS_FILE_NAME), "not json").unwrap();
        assert!(load_recent_projects(folder.path()).is_empty());
    }
}
