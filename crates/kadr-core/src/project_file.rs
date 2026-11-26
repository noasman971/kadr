//! The `.kadr` project file: JSON with a format version.
//!
//! ```json
//! {
//!   "format_version": 1,
//!   "project": { "settings": { … }, "media_assets": [ … ], "timeline": { … }, … }
//! }
//! ```
//!
//! When the format changes, [`CURRENT_FORMAT_VERSION`] goes up and a migration
//! is added to [`MIGRATIONS`]. Loading an older file runs every migration from
//! its version to the current one; loading a file from a **newer** Kadr is
//! refused with a clear message instead of losing data.
//!
//! Every loaded project is checked with [`validate_project`], so a damaged file
//! cannot put the editor in an invalid state.
//!
//! This module only converts between text and projects; reading and writing
//! files on disk is done by `kadr-engine`.

use serde::Serialize;
use serde_json::Value;

use crate::project::Project;
use crate::validation::{ValidationError, validate_project};

/// The format version written by this version of Kadr.
pub const CURRENT_FORMAT_VERSION: u32 = 1;

/// The recommended file extension, without the dot.
pub const PROJECT_FILE_EXTENSION: &str = "kadr";

/// Name of the version field at the top of the file.
const FORMAT_VERSION_FIELD: &str = "format_version";
/// Name of the field holding the project.
const PROJECT_FIELD: &str = "project";

/// Why a project file could not be read or written.
#[derive(Debug, thiserror::Error)]
pub enum ProjectFileError {
    /// The text is not valid JSON, or does not have the expected structure.
    #[error("the project file is damaged or is not a Kadr project: {0}")]
    MalformedFile(#[from] serde_json::Error),

    /// The file has no format version.
    #[error("the project file has no \"{FORMAT_VERSION_FIELD}\": it is not a Kadr project")]
    MissingFormatVersion,

    /// The file was written by a newer Kadr.
    #[error(
        "this project was saved by a newer version of Kadr (file format {file_version}, this version reads up to {CURRENT_FORMAT_VERSION}); please update Kadr"
    )]
    FormatTooNew {
        /// The version found in the file.
        file_version: u32,
    },

    /// The file's version is unknown (for example 0).
    #[error("project file format {file_version} is unknown")]
    UnknownFormatVersion {
        /// The version found in the file.
        file_version: u32,
    },

    /// A migration from an older format failed.
    #[error("could not upgrade the project from file format {from_version}: {reason}")]
    MigrationFailed {
        /// The version being upgraded.
        from_version: u32,
        /// What went wrong.
        reason: String,
    },

    /// The file was read but the project breaks a rule.
    #[error("the project file contains an invalid project: {0}")]
    InvalidProject(#[from] ValidationError),
}

/// Upgrades a project file from `from_version` to `from_version + 1`, by
/// rewriting its JSON.
#[derive(Debug, Clone, Copy)]
pub struct Migration {
    /// The format version this migration upgrades.
    pub from_version: u32,
    /// Rewrites the whole document (including `format_version`, which the
    /// loader updates afterwards).
    pub upgrade_document: fn(&mut Value) -> Result<(), String>,
}

/// Every migration, in increasing `from_version` order. Empty while the format
/// is still at version 1.
pub const MIGRATIONS: &[Migration] = &[];

/// The top of the file, as written.
#[derive(Serialize)]
struct ProjectFileContents<'project> {
    format_version: u32,
    project: &'project Project,
}

/// Writes a project as indented JSON, ready to save in a `.kadr` file.
///
/// # Errors
///
/// Only fails if a value cannot be represented in JSON (should never happen).
pub fn save_project_to_json(project: &Project) -> Result<String, ProjectFileError> {
    let contents = ProjectFileContents {
        format_version: CURRENT_FORMAT_VERSION,
        project,
    };
    Ok(serde_json::to_string_pretty(&contents)?)
}

/// Reads a project from the text of a `.kadr` file, upgrading older formats
/// and checking every rule.
///
/// # Errors
///
/// See [`ProjectFileError`]: damaged file, newer format, failed migration, or
/// invalid project.
pub fn load_project_from_json(json_text: &str) -> Result<Project, ProjectFileError> {
    let mut document: Value = serde_json::from_str(json_text)?;
    upgrade_document_format(&mut document, MIGRATIONS, CURRENT_FORMAT_VERSION)?;
    let project_value = document
        .get_mut(PROJECT_FIELD)
        .map(Value::take)
        .unwrap_or(Value::Null);
    let project: Project = serde_json::from_value(project_value)?;
    validate_project(&project)?;
    Ok(project)
}

/// Reads the format version and runs the migrations needed to reach
/// `target_version` (always [`CURRENT_FORMAT_VERSION`] outside tests).
fn upgrade_document_format(
    document: &mut Value,
    migrations: &[Migration],
    target_version: u32,
) -> Result<(), ProjectFileError> {
    let mut file_version = read_format_version(document)?;
    if file_version > target_version {
        return Err(ProjectFileError::FormatTooNew { file_version });
    }
    if file_version == 0 {
        return Err(ProjectFileError::UnknownFormatVersion { file_version });
    }
    while file_version < target_version {
        let migration = migrations
            .iter()
            .find(|migration| migration.from_version == file_version)
            .ok_or_else(|| ProjectFileError::MigrationFailed {
                from_version: file_version,
                reason: "no migration is defined for this version".to_string(),
            })?;
        (migration.upgrade_document)(document).map_err(|reason| {
            ProjectFileError::MigrationFailed {
                from_version: file_version,
                reason,
            }
        })?;
        file_version += 1;
        document[FORMAT_VERSION_FIELD] = Value::from(file_version);
    }
    Ok(())
}

fn read_format_version(document: &Value) -> Result<u32, ProjectFileError> {
    document
        .get(FORMAT_VERSION_FIELD)
        .and_then(Value::as_u64)
        .and_then(|version| u32::try_from(version).ok())
        .ok_or(ProjectFileError::MissingFormatVersion)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::ProjectSettings;

    #[test]
    fn an_empty_project_round_trips() {
        let project = Project::new(ProjectSettings::vertical_1080x1920());
        let json_text = save_project_to_json(&project).unwrap();
        assert!(json_text.contains("\"format_version\": 1"));
        assert_eq!(load_project_from_json(&json_text).unwrap(), project);
    }

    #[test]
    fn files_that_are_not_projects_are_refused() {
        assert!(matches!(
            load_project_from_json("not json"),
            Err(ProjectFileError::MalformedFile(_))
        ));
        assert!(matches!(
            load_project_from_json("{}"),
            Err(ProjectFileError::MissingFormatVersion)
        ));
        assert!(matches!(
            load_project_from_json(r#"{ "format_version": 1 }"#),
            Err(ProjectFileError::MalformedFile(_))
        ));
        assert!(matches!(
            load_project_from_json(r#"{ "format_version": 0, "project": {} }"#),
            Err(ProjectFileError::UnknownFormatVersion { file_version: 0 })
        ));
    }

    #[test]
    fn files_from_a_newer_kadr_are_refused_with_a_clear_message() {
        let error =
            load_project_from_json(r#"{ "format_version": 99, "project": {} }"#).unwrap_err();
        assert!(error.to_string().contains("newer version of Kadr"));
    }

    /// Simulates a future format 3, where version 1 files must be upgraded twice.
    #[test]
    fn migrations_run_in_order_up_to_the_target_version() {
        fn rename_field(document: &mut Value) -> Result<(), String> {
            let old_value = document["old_field"].take();
            document["renamed_field"] = old_value;
            Ok(())
        }
        fn double_field(document: &mut Value) -> Result<(), String> {
            let value = document["renamed_field"].as_u64().ok_or("missing field")?;
            document["renamed_field"] = Value::from(value * 2);
            Ok(())
        }
        let migrations = [
            Migration {
                from_version: 1,
                upgrade_document: rename_field,
            },
            Migration {
                from_version: 2,
                upgrade_document: double_field,
            },
        ];
        let mut document = serde_json::json!({ "format_version": 1, "old_field": 21 });
        upgrade_document_format(&mut document, &migrations, 3).unwrap();
        assert_eq!(document["renamed_field"], 42);
        assert_eq!(document["format_version"], 3);

        let mut document_needing_missing_migration = serde_json::json!({ "format_version": 1 });
        assert!(matches!(
            upgrade_document_format(&mut document_needing_missing_migration, &[], 2),
            Err(ProjectFileError::MigrationFailed {
                from_version: 1,
                ..
            })
        ));
    }

    #[test]
    fn a_failing_migration_is_reported() {
        fn always_fail(_: &mut Value) -> Result<(), String> {
            Err("field missing".to_string())
        }
        let migrations = [Migration {
            from_version: 1,
            upgrade_document: always_fail,
        }];
        let mut document = serde_json::json!({ "format_version": 1 });
        let error = upgrade_document_format(&mut document, &migrations, 2).unwrap_err();
        assert_eq!(
            error.to_string(),
            "could not upgrade the project from file format 1: field missing"
        );
    }
}
