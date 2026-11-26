//! Media tools: find files in the allowed folders, describe them, import them.

use std::fs;
use std::path::{Path, PathBuf};

use kadr_core::commands::{CreatedObjectsExt, ImportMedia};
use kadr_core::error::EditError;
use kadr_core::ids::MediaAssetId;
use kadr_core::project::Project;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{NoParameters, ToolEffect, ToolEntry, declare_tool};
use crate::output::{ToolError, ToolOutput};
use crate::toolbox::KadrToolbox;
use crate::views::MediaView;

/// The media tools.
pub(crate) fn media_tools() -> Vec<ToolEntry> {
    vec![
        declare_tool(
            "find_media_files",
            "Find video, audio and image files",
            FIND_MEDIA_FILES_DESCRIPTION,
            ToolEffect::ReadsOnly,
            find_media_files,
        ),
        declare_tool(
            "probe_media",
            "Describe a media file",
            PROBE_MEDIA_DESCRIPTION,
            ToolEffect::ReadsOnly,
            probe_media,
        ),
        declare_tool(
            "import_media",
            "Import media files into the project",
            IMPORT_MEDIA_DESCRIPTION,
            ToolEffect::ChangesProject,
            import_media,
        ),
        declare_tool(
            "list_media",
            "List the project's media",
            LIST_MEDIA_DESCRIPTION,
            ToolEffect::ReadsOnly,
            list_media,
        ),
    ]
}

// ----- find_media_files -----

const FIND_MEDIA_FILES_DESCRIPTION: &str = "\
Searches the folders the user allowed for video, audio and image files, and returns their full \
paths. Use it to find the files the user talks about: videos attached to the conversation are \
NOT available to Kadr, only files on the user's computer.

Examples: {} lists media in every allowed folder; {\"name_contains\": \"interview\"}; \
{\"folder\": \"C:\\\\Users\\\\sam\\\\Videos\\\\Holidays\", \"kind\": \"video\"}.";

/// Parameters of `find_media_files`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct FindMediaFilesParameters {
    /// Folder to search (full path, inside an allowed folder). Default: every allowed folder.
    folder: Option<PathBuf>,
    /// Keep only files whose name contains this text (not case sensitive). Example: "interview".
    name_contains: Option<String>,
    /// Keep only this kind of file.
    kind: Option<FileKindFilter>,
    /// Also search sub-folders. Default true.
    #[serde(default = "default_true")]
    include_subfolders: bool,
    /// Largest number of files returned (1 to 500). Default 100.
    maximum_results: Option<usize>,
}

fn default_true() -> bool {
    true
}

/// A kind of media file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum FileKindFilter {
    /// Videos: mp4, mov, mkv, webm, avi, m4v…
    Video,
    /// Sound: mp3, wav, m4a, aac, flac, ogg…
    Audio,
    /// Still pictures: png, jpg, jpeg, webp, bmp…
    Image,
}

/// File extensions Kadr recognises, by kind.
const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "mov", "mkv", "webm", "avi", "m4v", "mts", "m2ts", "wmv", "flv", "3gp",
];
const AUDIO_EXTENSIONS: &[&str] = &["mp3", "wav", "m4a", "aac", "flac", "ogg", "opus", "wma"];
const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp", "bmp", "gif", "tif", "tiff"];

/// Sub-folders deeper than this are not searched, to keep the answer quick.
const MAXIMUM_FOLDER_DEPTH: usize = 8;
/// Default and largest number of results.
const DEFAULT_RESULT_COUNT: usize = 100;
const MAXIMUM_RESULT_COUNT: usize = 500;
const BYTES_PER_MEGABYTE: f64 = 1_000_000.0;

#[derive(Debug, Serialize)]
struct FoundFileView {
    file_path: String,
    kind: &'static str,
    size_in_megabytes: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    already_imported_as_media_id: Option<u64>,
}

/// The search in progress.
struct FileSearch<'search> {
    name_filter: Option<String>,
    kind_filter: Option<FileKindFilter>,
    include_subfolders: bool,
    maximum_results: usize,
    found: Vec<(PathBuf, FileKindFilter, u64)>,
    is_truncated: bool,
    project: Option<&'search Project>,
}

impl FileSearch<'_> {
    fn search_folder(&mut self, folder: &Path, depth: usize) {
        let Ok(entries) = fs::read_dir(folder) else {
            return;
        };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            if self.found.len() >= self.maximum_results {
                self.is_truncated = true;
                return;
            }
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            let is_hidden = entry.file_name().to_string_lossy().starts_with('.');
            if file_type.is_dir() {
                if self.include_subfolders && !is_hidden && depth < MAXIMUM_FOLDER_DEPTH {
                    self.search_folder(&path, depth + 1);
                }
            } else if file_type.is_file() && !is_hidden {
                self.consider_file(&path, &entry);
            }
        }
    }

    fn consider_file(&mut self, path: &Path, entry: &fs::DirEntry) {
        let Some(kind) = kind_of_file(path) else {
            return;
        };
        if self.kind_filter.is_some_and(|wanted| wanted != kind) {
            return;
        }
        if let Some(name_filter) = &self.name_filter {
            let file_name = entry.file_name().to_string_lossy().to_lowercase();
            if !file_name.contains(name_filter.as_str()) {
                return;
            }
        }
        let size_in_bytes = entry.metadata().map(|metadata| metadata.len()).unwrap_or(0);
        self.found.push((path.to_path_buf(), kind, size_in_bytes));
    }

    fn views(&self) -> Vec<FoundFileView> {
        self.found
            .iter()
            .map(|(path, kind, size_in_bytes)| FoundFileView {
                file_path: path.display().to_string(),
                kind: match kind {
                    FileKindFilter::Video => "video",
                    FileKindFilter::Audio => "audio",
                    FileKindFilter::Image => "image",
                },
                size_in_megabytes: (*size_in_bytes as f64 / BYTES_PER_MEGABYTE * 10.0).round()
                    / 10.0,
                already_imported_as_media_id: self
                    .project
                    .and_then(|project| imported_media_with_path(project, path))
                    .map(MediaAssetId::raw),
            })
            .collect()
    }
}

/// The kind of a file, from its extension.
fn kind_of_file(path: &Path) -> Option<FileKindFilter> {
    let extension = path.extension()?.to_str()?.to_lowercase();
    if VIDEO_EXTENSIONS.contains(&extension.as_str()) {
        Some(FileKindFilter::Video)
    } else if AUDIO_EXTENSIONS.contains(&extension.as_str()) {
        Some(FileKindFilter::Audio)
    } else if IMAGE_EXTENSIONS.contains(&extension.as_str()) {
        Some(FileKindFilter::Image)
    } else {
        None
    }
}

/// The media of the project read from `path`, if it was imported already.
fn imported_media_with_path(project: &Project, path: &Path) -> Option<MediaAssetId> {
    project
        .media_assets()
        .iter()
        .find(|asset| asset.description.file_path == path)
        .map(|asset| asset.id)
}

fn find_media_files(
    toolbox: &mut KadrToolbox,
    parameters: FindMediaFilesParameters,
) -> Result<ToolOutput, ToolError> {
    let folders = match &parameters.folder {
        Some(folder) => vec![toolbox.allowed_folders().check_existing_folder(folder)?],
        None => toolbox.allowed_folders().folders().to_vec(),
    };
    if folders.is_empty() {
        return Err(ToolError::new(format!(
            "no file can be searched: {}",
            toolbox.allowed_folders().no_folder_explanation()
        )));
    }
    let mut search = FileSearch {
        name_filter: parameters.name_contains.map(|text| text.to_lowercase()),
        kind_filter: parameters.kind,
        include_subfolders: parameters.include_subfolders,
        maximum_results: parameters
            .maximum_results
            .unwrap_or(DEFAULT_RESULT_COUNT)
            .clamp(1, MAXIMUM_RESULT_COUNT),
        found: Vec::new(),
        is_truncated: false,
        project: toolbox.open_project().map(|open| open.project()),
    };
    for folder in &folders {
        search.search_folder(folder, 0);
    }
    let views = search.views();
    let mut message = format!("Found {} media file(s).", views.len());
    if search.is_truncated {
        message
            .push_str(" More files exist: narrow the search with name_contains, kind or folder.");
    }
    if views.is_empty() {
        message.push_str(&format!(
            " Searched: {}. Ask the user where the file is, or to allow its folder.",
            toolbox.allowed_folders().describe()
        ));
    }
    Ok(ToolOutput::message_with_details(message, &views))
}

// ----- probe_media -----

const PROBE_MEDIA_DESCRIPTION: &str = "\
Describes a media file without importing it: kind (video, audio, image), duration in seconds, \
picture size in pixels, frames per second and whether it has sound. \
Example: {\"file_path\": \"C:\\\\Users\\\\sam\\\\Videos\\\\beach.mp4\"}";

/// Parameters of `probe_media`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ProbeMediaParameters {
    /// Full path of the file, inside a folder the user allowed.
    file_path: PathBuf,
}

fn probe_media(
    toolbox: &mut KadrToolbox,
    parameters: ProbeMediaParameters,
) -> Result<ToolOutput, ToolError> {
    let file_path = toolbox
        .allowed_folders()
        .check_existing_file(&parameters.file_path)?;
    let description = toolbox.backend().probe(&file_path)?;
    Ok(ToolOutput::message_with_details(
        "The file:",
        &MediaView::of_description(&description),
    ))
}

// ----- import_media -----

const IMPORT_MEDIA_DESCRIPTION: &str = "\
Imports media files (videos, sounds, images) into the open project and returns their media_id. \
Importing does not place them on the timeline: use add_clip for that. A file already imported \
is not imported twice (its existing media_id is returned). All files are imported in one undo \
step. Example: {\"file_paths\": [\"C:\\\\Users\\\\sam\\\\Videos\\\\beach.mp4\", \
\"C:\\\\Users\\\\sam\\\\Music\\\\song.mp3\"]}";

/// Parameters of `import_media`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ImportMediaParameters {
    /// Full paths of the files, inside folders the user allowed (find them with find_media_files).
    file_paths: Vec<PathBuf>,
}

fn import_media(
    toolbox: &mut KadrToolbox,
    parameters: ImportMediaParameters,
) -> Result<ToolOutput, ToolError> {
    if parameters.file_paths.is_empty() {
        return Err(ToolError::new("give at least one file in file_paths"));
    }
    let project = toolbox.require_project()?;
    let mut already_imported = Vec::new();
    let mut descriptions = Vec::new();
    for requested_path in &parameters.file_paths {
        let file_path = toolbox
            .allowed_folders()
            .check_existing_file(requested_path)?;
        if let Some(media_id) = imported_media_with_path(project, &file_path) {
            already_imported.push(media_id);
            continue;
        }
        if descriptions.iter().any(
            |description: &kadr_core::media_asset::MediaAssetDescription| {
                description.file_path == file_path
            },
        ) {
            continue;
        }
        descriptions.push(toolbox.backend().probe(&file_path)?);
    }
    let label = format!("Import {} media file(s)", descriptions.len());
    let imported_ids = toolbox.change_project(&label, |transaction| {
        let mut imported_ids = Vec::new();
        for description in descriptions {
            let created = transaction.execute(ImportMedia::new(description))?;
            let media_id =
                created
                    .first_media_asset_id()
                    .ok_or_else(|| EditError::InternalInconsistency {
                        details: "an import did not report its media".to_string(),
                    })?;
            imported_ids.push(media_id);
        }
        Ok(imported_ids)
    })?;
    let project = toolbox.require_project()?;
    let views: Vec<MediaView> = imported_ids
        .iter()
        .chain(&already_imported)
        .filter_map(|media_id| project.media_asset(*media_id))
        .map(|asset| MediaView::of_asset(project, asset))
        .collect();
    let message = format!(
        "Imported {} file(s){}. Place them on the timeline with add_clip.",
        imported_ids.len(),
        if already_imported.is_empty() {
            String::new()
        } else {
            format!(" ({} were already imported)", already_imported.len())
        }
    );
    Ok(ToolOutput::message_with_details(message, &views))
}

// ----- list_media -----

const LIST_MEDIA_DESCRIPTION: &str = "\
Lists the media files imported in the open project, with their media_id, kind, duration, \
picture size, whether they have sound, and the clips that use them.";

fn list_media(toolbox: &mut KadrToolbox, _: NoParameters) -> Result<ToolOutput, ToolError> {
    let project = toolbox.require_project()?;
    let views: Vec<MediaView> = project
        .media_assets()
        .iter()
        .map(|asset| MediaView::of_asset(project, asset))
        .collect();
    let message = if views.is_empty() {
        "The project has no media yet: import files with import_media.".to_string()
    } else {
        format!("The project has {} media file(s):", views.len())
    };
    Ok(ToolOutput::message_with_details(message, &views))
}
