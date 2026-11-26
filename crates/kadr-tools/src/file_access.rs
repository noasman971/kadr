//! Which files the tools may touch: only those inside the folders the user allowed.
//!
//! Claude chooses paths, so every path it gives is checked here before Kadr reads
//! or writes anything:
//!
//! - the path must be **absolute**;
//! - it is resolved to its canonical form (symbolic links followed, `..`
//!   removed), and that form must lie inside one of the allowed folders, so a
//!   link or a `..` cannot lead outside;
//! - a file is only written if it does not exist yet, unless the caller asked to
//!   overwrite it explicitly;
//! - nothing here (and nothing in the tools) ever deletes a file.
//!
//! How the user gives the allowed folders is decided in decision record 0011:
//! `kadr-mcp --allowed-folder <folder>` in the Claude configuration.

use std::fmt::Write as _;
use std::path::{Component, Path, PathBuf};

use crate::output::ToolError;

/// The folders inside which tools may read and write files.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AllowedFolders {
    /// Canonical paths of the allowed folders.
    folders: Vec<PathBuf>,
    /// Folders the user gave that cannot be used, with the reason, so messages
    /// can point at a typing mistake in the configuration.
    rejected_folders: Vec<String>,
}

impl AllowedFolders {
    /// Allows the given folders (and everything inside them). A folder that does
    /// not exist or is not a folder is left out and reported by
    /// [`Self::rejected_folders`] and in error messages, so a typing mistake in
    /// the configuration is explained instead of stopping the server.
    pub fn new(folders: impl IntoIterator<Item = PathBuf>) -> Self {
        let mut allowed = Self::default();
        for folder in folders {
            match dunce::canonicalize(&folder) {
                Ok(canonical_folder) if canonical_folder.is_dir() => {
                    if !allowed.folders.contains(&canonical_folder) {
                        allowed.folders.push(canonical_folder);
                    }
                }
                Ok(_) => allowed
                    .rejected_folders
                    .push(format!("\"{}\" is a file, not a folder", folder.display())),
                Err(error) => allowed
                    .rejected_folders
                    .push(format!("\"{}\" cannot be used: {error}", folder.display())),
            }
        }
        allowed
    }

    /// The allowed folders, canonical.
    pub fn folders(&self) -> &[PathBuf] {
        &self.folders
    }

    /// The folders given that cannot be used, each with the reason.
    pub fn rejected_folders(&self) -> &[String] {
        &self.rejected_folders
    }

    /// Checks that `path` is an existing file inside an allowed folder.
    /// Returns its canonical path.
    ///
    /// # Errors
    ///
    /// Fails when the path is relative, does not exist, is a folder, or is
    /// outside the allowed folders.
    pub fn check_existing_file(&self, path: &Path) -> Result<PathBuf, ToolError> {
        let canonical_path = self.canonical_existing_path(path)?;
        if canonical_path.is_dir() {
            return Err(ToolError::new(format!(
                "\"{}\" is a folder, not a file",
                path.display()
            )));
        }
        Ok(canonical_path)
    }

    /// Checks that `path` is an existing folder inside an allowed folder.
    /// Returns its canonical path.
    ///
    /// # Errors
    ///
    /// Fails when the path is relative, does not exist, is not a folder, or is
    /// outside the allowed folders.
    pub fn check_existing_folder(&self, path: &Path) -> Result<PathBuf, ToolError> {
        let canonical_path = self.canonical_existing_path(path)?;
        if !canonical_path.is_dir() {
            return Err(ToolError::new(format!(
                "\"{}\" is a file, not a folder",
                path.display()
            )));
        }
        Ok(canonical_path)
    }

    /// Checks that a file may be written at `path`: its folder exists inside an
    /// allowed folder, and the file does not exist yet unless `overwrite` is true.
    /// Returns the canonical path the file will have.
    ///
    /// # Errors
    ///
    /// Fails when the path is relative, its folder does not exist or is outside
    /// the allowed folders, it names a folder, or the file exists and
    /// `overwrite` is false.
    pub fn check_file_to_write(&self, path: &Path, overwrite: bool) -> Result<PathBuf, ToolError> {
        require_absolute(path)?;
        let file_name = match path.components().next_back() {
            Some(Component::Normal(file_name)) => file_name.to_owned(),
            _ => {
                return Err(ToolError::new(format!(
                    "\"{}\" does not end with a file name",
                    path.display()
                )));
            }
        };
        let parent_folder = path.parent().unwrap_or(path);
        let canonical_folder = self.check_existing_folder(parent_folder)?;
        let canonical_path = canonical_folder.join(file_name);
        if canonical_path.is_dir() {
            return Err(ToolError::new(format!(
                "\"{}\" is a folder, not a file",
                path.display()
            )));
        }
        if canonical_path.exists() && !overwrite {
            return Err(ToolError::new(format!(
                "\"{}\" already exists. Kadr never replaces a file unless asked: choose another \
                 name, or call again with \"overwrite\": true if the user agrees to replace it",
                canonical_path.display()
            )));
        }
        Ok(canonical_path)
    }

    /// Whether `canonical_path` lies inside an allowed folder.
    pub fn contains(&self, canonical_path: &Path) -> bool {
        self.folders
            .iter()
            .any(|folder| canonical_path.starts_with(folder))
    }

    /// The canonical form of an existing `path`, checked against the allowed folders.
    fn canonical_existing_path(&self, path: &Path) -> Result<PathBuf, ToolError> {
        require_absolute(path)?;
        let canonical_path = dunce::canonicalize(path).map_err(|error| {
            ToolError::new(format!("\"{}\" cannot be opened: {error}", path.display()))
        })?;
        if !self.contains(&canonical_path) {
            return Err(self.outside_error(path));
        }
        Ok(canonical_path)
    }

    /// The error for a path outside every allowed folder.
    fn outside_error(&self, path: &Path) -> ToolError {
        if self.folders.is_empty() {
            return ToolError::new(format!(
                "\"{}\" cannot be used: {}",
                path.display(),
                self.no_folder_explanation()
            ));
        }
        ToolError::new(format!(
            "\"{}\" is outside the folders the user allowed Kadr to use ({}). Use a file inside \
             them, or ask the user to add its folder with another \"--allowed-folder\" argument \
             in their Claude configuration{}",
            path.display(),
            self.describe(),
            self.rejected_folders_note()
        ))
    }

    /// Why no file can be used yet, and what the user should do.
    pub fn no_folder_explanation(&self) -> String {
        format!(
            "the user has not allowed Kadr to use any folder yet. Ask them to add \
             \"--allowed-folder\" followed by a folder (for example their Videos folder) to the \
             arguments of the Kadr server in their Claude configuration, then restart Claude \
             (see the Kadr README){}",
            self.rejected_folders_note()
        )
    }

    /// A sentence about the folders given that cannot be used, if any.
    fn rejected_folders_note(&self) -> String {
        if self.rejected_folders.is_empty() {
            return String::new();
        }
        format!(
            ". Note: some folders given with --allowed-folder cannot be used: {}",
            self.rejected_folders.join("; ")
        )
    }

    /// The allowed folders, as a list for messages.
    pub fn describe(&self) -> String {
        if self.folders.is_empty() {
            return "none".to_string();
        }
        let mut description = String::new();
        for (index, folder) in self.folders.iter().enumerate() {
            if index > 0 {
                description.push_str(", ");
            }
            let _ = write!(description, "\"{}\"", folder.display());
        }
        description
    }
}

/// Refuses relative paths: their meaning would depend on where Claude started Kadr.
fn require_absolute(path: &Path) -> Result<(), ToolError> {
    if path.is_absolute() {
        return Ok(());
    }
    Err(ToolError::new(format!(
        "\"{}\" is not a full path. Give the complete path, for example \
         \"C:\\Users\\name\\Videos\\holiday.mp4\" on Windows or \"/home/name/Videos/holiday.mp4\"",
        path.display()
    )))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    struct Folders {
        allowed: tempfile::TempDir,
        outside: tempfile::TempDir,
    }

    fn folders() -> (Folders, AllowedFolders) {
        let allowed = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let allowed_folders = AllowedFolders::new([allowed.path().to_path_buf()]);
        (Folders { allowed, outside }, allowed_folders)
    }

    #[test]
    fn files_inside_an_allowed_folder_are_accepted() {
        let (folders, allowed_folders) = folders();
        let nested = folders.allowed.path().join("holidays");
        fs::create_dir(&nested).unwrap();
        let video = nested.join("beach.mp4");
        fs::write(&video, b"not really a video").unwrap();

        let checked = allowed_folders.check_existing_file(&video).unwrap();
        assert!(checked.ends_with("beach.mp4"));
        assert!(allowed_folders.check_existing_folder(&nested).is_ok());
    }

    #[test]
    fn files_outside_are_refused_with_the_allowed_folders_named() {
        let (folders, allowed_folders) = folders();
        let secret = folders.outside.path().join("secret.mp4");
        fs::write(&secret, b"secret").unwrap();

        let error = allowed_folders.check_existing_file(&secret).unwrap_err();
        assert!(error.to_string().contains("outside the folders"), "{error}");
        assert!(error.to_string().contains("--allowed-folder"), "{error}");
    }

    #[test]
    fn dot_dot_cannot_escape_an_allowed_folder() {
        let (folders, allowed_folders) = folders();
        let secret = folders.outside.path().join("secret.mp4");
        fs::write(&secret, b"secret").unwrap();
        let outside_name = folders.outside.path().file_name().unwrap();
        let sneaky = folders
            .allowed
            .path()
            .join("..")
            .join(outside_name)
            .join("secret.mp4");
        assert!(allowed_folders.check_existing_file(&sneaky).is_err());
        assert!(
            allowed_folders
                .check_file_to_write(&folders.allowed.path().join("..").join("x.kadr"), false)
                .is_err()
        );
    }

    #[test]
    fn relative_paths_are_refused() {
        let (_folders, allowed_folders) = folders();
        let error = allowed_folders
            .check_existing_file(Path::new("video.mp4"))
            .unwrap_err();
        assert!(error.to_string().contains("not a full path"), "{error}");
    }

    #[test]
    fn existing_files_are_only_replaced_when_asked() {
        let (folders, allowed_folders) = folders();
        let export = folders.allowed.path().join("export.mp4");
        assert!(allowed_folders.check_file_to_write(&export, false).is_ok());

        fs::write(&export, b"previous export").unwrap();
        let error = allowed_folders
            .check_file_to_write(&export, false)
            .unwrap_err();
        assert!(error.to_string().contains("\"overwrite\": true"), "{error}");
        assert!(allowed_folders.check_file_to_write(&export, true).is_ok());
    }

    #[test]
    fn writing_needs_an_existing_folder() {
        let (folders, allowed_folders) = folders();
        let missing_folder = folders.allowed.path().join("missing").join("x.kadr");
        assert!(
            allowed_folders
                .check_file_to_write(&missing_folder, false)
                .is_err()
        );
    }

    #[test]
    fn without_any_allowed_folder_the_message_says_how_to_add_one() {
        let folder = tempfile::tempdir().unwrap();
        let file = folder.path().join("a.mp4");
        fs::write(&file, b"a").unwrap();
        let error = AllowedFolders::default()
            .check_existing_file(&file)
            .unwrap_err();
        assert!(error.to_string().contains("not allowed Kadr"), "{error}");
    }

    #[test]
    fn a_missing_allowed_folder_is_left_out_and_explained() {
        let folder = tempfile::tempdir().unwrap();
        let allowed_folders = AllowedFolders::new([folder.path().join("missing")]);
        assert!(allowed_folders.folders().is_empty());
        assert_eq!(allowed_folders.rejected_folders().len(), 1);

        let file = folder.path().join("a.mp4");
        fs::write(&file, b"a").unwrap();
        let error = allowed_folders.check_existing_file(&file).unwrap_err();
        assert!(error.to_string().contains("missing"), "{error}");
    }
}
