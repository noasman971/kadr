//! The folders the user allowed Claude to use, kept in Kadr's settings
//! (decision record 0031).
//!
//! The desktop app's *Connect to Claude* screen writes them to
//! `claude-access.json` in its configuration folder. `kadr-mcp`, given the
//! file's path (`--kadr-settings`), reads it before every call, and the app
//! reads it for the calls it runs: a folder added or removed applies to the next
//! call, without restarting Claude Desktop. Folders given with
//! `--allowed-folder` still count.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The name of the settings file, in the app's configuration folder.
pub const CLAUDE_ACCESS_FILE_NAME: &str = "claude-access.json";

/// What `claude-access.json` holds.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaudeAccessSettings {
    /// The folders Claude may read and write in, as the user chose them.
    #[serde(default)]
    pub allowed_folders: Vec<PathBuf>,
}

impl ClaudeAccessSettings {
    /// The settings saved in `file_path`; empty when the file does not exist yet.
    ///
    /// # Errors
    ///
    /// When the file exists but cannot be read or is not valid.
    pub fn load(file_path: &Path) -> io::Result<Self> {
        match fs::read_to_string(file_path) {
            Ok(text) => serde_json::from_str(&text).map_err(io::Error::other),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error),
        }
    }

    /// Saves the settings to `file_path` (through a temporary file, so a crash
    /// never leaves half a file), creating its folder if needed.
    ///
    /// # Errors
    ///
    /// When the file cannot be written.
    pub fn save(&self, file_path: &Path) -> io::Result<()> {
        if let Some(folder) = file_path.parent() {
            fs::create_dir_all(folder)?;
        }
        let text = serde_json::to_string_pretty(self).map_err(io::Error::other)?;
        let temporary_path = file_path.with_extension("json.saving");
        fs::write(&temporary_path, text)?;
        fs::rename(&temporary_path, file_path)
    }

    /// Adds `folder` unless it is already listed.
    pub fn add_folder(&mut self, folder: PathBuf) {
        if !self.allowed_folders.contains(&folder) {
            self.allowed_folders.push(folder);
        }
    }

    /// Removes `folder` from the list.
    pub fn remove_folder(&mut self, folder: &Path) {
        self.allowed_folders.retain(|listed| listed != folder);
    }
}

/// The folders given on the command line followed by those of the settings file
/// (when there is one and it can be read), without repeats.
pub fn combined_allowed_folders(
    argument_folders: &[PathBuf],
    settings_file: Option<&Path>,
) -> Vec<PathBuf> {
    let mut folders = argument_folders.to_vec();
    if let Some(settings) = settings_file.and_then(|path| ClaudeAccessSettings::load(path).ok()) {
        for folder in settings.allowed_folders {
            if !folders.contains(&folder) {
                folders.push(folder);
            }
        }
    }
    folders
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_are_saved_and_read_back() {
        let directory = tempfile::tempdir().unwrap();
        let file_path = directory
            .path()
            .join("settings")
            .join(CLAUDE_ACCESS_FILE_NAME);
        assert_eq!(
            ClaudeAccessSettings::load(&file_path).unwrap(),
            ClaudeAccessSettings::default()
        );
        let mut settings = ClaudeAccessSettings::default();
        settings.add_folder(PathBuf::from("C:/Videos"));
        settings.add_folder(PathBuf::from("D:/Edits"));
        settings.add_folder(PathBuf::from("C:/Videos"));
        settings.save(&file_path).unwrap();
        let mut read_back = ClaudeAccessSettings::load(&file_path).unwrap();
        assert_eq!(read_back, settings);
        assert_eq!(read_back.allowed_folders.len(), 2);
        read_back.remove_folder(Path::new("C:/Videos"));
        assert_eq!(read_back.allowed_folders, vec![PathBuf::from("D:/Edits")]);
    }

    #[test]
    fn argument_folders_come_first_and_a_broken_file_is_ignored() {
        let directory = tempfile::tempdir().unwrap();
        let file_path = directory.path().join(CLAUDE_ACCESS_FILE_NAME);
        let settings = ClaudeAccessSettings {
            allowed_folders: vec![PathBuf::from("C:/Videos"), PathBuf::from("D:/Edits")],
        };
        settings.save(&file_path).unwrap();
        assert_eq!(
            combined_allowed_folders(&[PathBuf::from("C:/Videos")], Some(&file_path)),
            vec![PathBuf::from("C:/Videos"), PathBuf::from("D:/Edits")]
        );
        fs::write(&file_path, "not json").unwrap();
        assert_eq!(
            combined_allowed_folders(&[PathBuf::from("C:/Videos")], Some(&file_path)),
            vec![PathBuf::from("C:/Videos")]
        );
    }
}
