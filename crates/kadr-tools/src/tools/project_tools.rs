//! Project tools: create, open, save, describe, change settings, undo and redo.

use std::path::PathBuf;

use kadr_core::commands::ChangeProjectSettings;
use kadr_core::project::{Project, ProjectSettings};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{NoParameters, ToolEffect, ToolEntry, declare_tool};
use crate::output::{ToolError, ToolOutput};
use crate::time_input::seconds_in;
use crate::toolbox::ToolCall;
use crate::tools::common::{CanvasPreset, SettingsChanges, with_extension};
use crate::tools::picture_tools::{check_stored_color_lut_file, enabled_color_luts};
use crate::views::SettingsView;

/// The project tools.
pub(crate) fn project_tools() -> Vec<ToolEntry> {
    vec![
        declare_tool(
            "get_project_info",
            "Describe the open project",
            GET_PROJECT_INFO_DESCRIPTION,
            ToolEffect::ReadsOnly,
            get_project_info,
        ),
        declare_tool(
            "create_project",
            "Create a project",
            CREATE_PROJECT_DESCRIPTION,
            ToolEffect::WritesFiles,
            create_project,
        ),
        declare_tool(
            OPEN_PROJECT_TOOL_NAME,
            "Open a project",
            OPEN_PROJECT_DESCRIPTION,
            ToolEffect::ReadsOnly,
            open_project,
        ),
        declare_tool(
            "save_project",
            "Save the project",
            SAVE_PROJECT_DESCRIPTION,
            ToolEffect::WritesFiles,
            save_project,
        ),
        declare_tool(
            "set_project_settings",
            "Change the video size, frame rate or background",
            SET_PROJECT_SETTINGS_DESCRIPTION,
            ToolEffect::ChangesProject,
            set_project_settings,
        ),
        declare_tool(
            "undo",
            "Undo",
            UNDO_DESCRIPTION,
            ToolEffect::ChangesProject,
            undo,
        ),
        declare_tool(
            "redo",
            "Redo",
            REDO_DESCRIPTION,
            ToolEffect::ChangesProject,
            redo,
        ),
    ]
}

// ----- get_project_info -----

const GET_PROJECT_INFO_DESCRIPTION: &str = "\
Describes the open Kadr project: its file, video size (canvas) and frame rate, duration, \
number of tracks, clips and media files, what undo and redo would do, and the folders the \
user allowed Kadr to use. Call it first in a conversation: if no project is open it says so \
and lists the allowed folders.

Kadr is a video editor running on the user's computer. Videos attached to this conversation \
are NOT available to Kadr: the user names their files (or their folder) and Kadr finds them \
with find_media_files. Times are in seconds, sizes and positions in pixels with the origin at \
the top-left corner.";

#[derive(Debug, Serialize)]
struct ProjectInfoView {
    project_file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    settings: Option<SettingsView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration_in_seconds: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    track_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    clip_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    media_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    undo_would_revert: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    redo_would_reapply: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    recent_changes_oldest_first: Vec<String>,
    allowed_folders: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    unusable_allowed_folders: Vec<String>,
}

/// How many history steps `get_project_info` lists.
const LISTED_HISTORY_STEPS: usize = 10;

fn get_project_info(toolbox: &mut ToolCall<'_>, _: NoParameters) -> Result<ToolOutput, ToolError> {
    let allowed_folders: Vec<String> = toolbox
        .allowed_folders()
        .folders()
        .iter()
        .map(|folder| folder.display().to_string())
        .collect();
    let unusable_allowed_folders = toolbox.allowed_folders().rejected_folders().to_vec();
    let Some(project_session) = toolbox.open_project() else {
        let view = ProjectInfoView {
            project_file: None,
            settings: None,
            duration_in_seconds: None,
            track_count: None,
            clip_count: None,
            media_count: None,
            undo_would_revert: None,
            redo_would_reapply: None,
            recent_changes_oldest_first: Vec::new(),
            allowed_folders,
            unusable_allowed_folders,
        };
        return Ok(ToolOutput::message_with_details(
            "No project is open. Call open_project with the path of a .kadr file, or \
             create_project to start one.",
            &view,
        ));
    };
    let project = project_session.project();
    let labels = project_session.undo_history_labels();
    let recent_changes = labels[labels.len().saturating_sub(LISTED_HISTORY_STEPS)..].to_vec();
    let view = ProjectInfoView {
        project_file: Some(project_session.file_path().display().to_string()),
        settings: Some(SettingsView::of(project.settings())),
        duration_in_seconds: Some(seconds_in(project.timeline().duration())),
        track_count: Some(project.timeline().tracks().len()),
        clip_count: Some(project.timeline().clip_ids().len()),
        media_count: Some(project.media_assets().len()),
        undo_would_revert: project_session.next_undo_label(),
        redo_would_reapply: project_session.next_redo_label(),
        recent_changes_oldest_first: recent_changes,
        allowed_folders,
        unusable_allowed_folders,
    };
    Ok(ToolOutput::message_with_details("The open project:", &view))
}

// ----- create_project -----

const CREATE_PROJECT_DESCRIPTION: &str = "\
Creates a new, empty Kadr project file (.kadr) and opens it: the next tools edit it, and every \
change is saved to this file automatically. Ask the user where to save it if they have not \
said; the folder must be one they allowed (see get_project_info). Never replaces an existing \
file unless \"overwrite\": true (ask the user first).

The canvas is the size of the exported video: vertical_1080x1920 for TikTok, Instagram Reels \
and YouTube Shorts; landscape_1920x1080 for YouTube (the default).

Example: {\"file_path\": \"C:\\\\Users\\\\sam\\\\Videos\\\\holiday.kadr\", \"canvas\": \
\"vertical_1080x1920\"}";

/// Parameters of `create_project`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CreateProjectParameters {
    /// Full path of the project file to create, inside a folder the user allowed. ".kadr" is
    /// added if there is no extension. Example: "C:\\Users\\sam\\Videos\\holiday.kadr".
    file_path: PathBuf,
    /// Standard video size. Default: landscape_1920x1080.
    canvas: Option<CanvasPreset>,
    /// Custom canvas width, in pixels (even number from 16 to 8192). Give height_in_pixels too.
    width_in_pixels: Option<u32>,
    /// Custom canvas height, in pixels (even number from 16 to 8192). Give width_in_pixels too.
    height_in_pixels: Option<u32>,
    /// Frames per second of the video. Default 30. Examples: 24, 25, 30, 60, 29.97.
    frames_per_second: Option<f64>,
    /// Replace the file if it already exists. Only true if the user agreed. Default false.
    #[serde(default)]
    overwrite: bool,
}

fn create_project(
    toolbox: &mut ToolCall<'_>,
    parameters: CreateProjectParameters,
) -> Result<ToolOutput, ToolError> {
    let file_path = with_extension(&parameters.file_path, "kadr")?;
    let file_path = toolbox
        .allowed_folders()
        .check_file_to_write(&file_path, parameters.overwrite)?;
    let settings = SettingsChanges {
        canvas: parameters.canvas,
        width_in_pixels: parameters.width_in_pixels,
        height_in_pixels: parameters.height_in_pixels,
        frames_per_second: parameters.frames_per_second,
        background_color: None,
    }
    .applied_to(ProjectSettings::landscape_1080p())?;
    toolbox
        .host_mut()
        .create_project(Project::new(settings), file_path)?;
    let project_session = toolbox.require_project_session()?;
    let message = format!(
        "Created and opened \"{}\". It is empty: import media with import_media (find files \
         with find_media_files), then place them with add_clip.",
        project_session.file_path().display()
    );
    let settings_view = SettingsView::of(project_session.project().settings());
    Ok(ToolOutput::message_with_details(message, &settings_view))
}

// ----- open_project -----

/// The name of the tool that opens another project: it only reads files, but
/// changes which project the host has open.
pub(crate) const OPEN_PROJECT_TOOL_NAME: &str = "open_project";

const OPEN_PROJECT_DESCRIPTION: &str = "\
Opens an existing Kadr project file (.kadr) so the next tools edit it. Every change is then \
saved to this file automatically. The project that was open before (already saved) is closed. \
Example: {\"file_path\": \"C:\\\\Users\\\\sam\\\\Videos\\\\holiday.kadr\"}";

/// Parameters of `open_project`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct OpenProjectParameters {
    /// Full path of the .kadr file, inside a folder the user allowed.
    file_path: PathBuf,
}

fn open_project(
    toolbox: &mut ToolCall<'_>,
    parameters: OpenProjectParameters,
) -> Result<ToolOutput, ToolError> {
    let file_path = toolbox
        .allowed_folders()
        .check_existing_file(&parameters.file_path)?;
    toolbox.host_mut().open_project_file(file_path)?;
    let unreadable_media = media_outside_allowed_folders(toolbox, toolbox.require_project()?);
    let unreadable_luts = color_luts_outside_allowed_folders(toolbox, toolbox.require_project()?);
    let mut output = get_project_info(toolbox, NoParameters {})?;
    if !unreadable_media.is_empty() {
        output = ToolOutput::text(format!(
            "{}\n\nWarning: media {} are outside the allowed folders, so Kadr will refuse to \
             read them (pictures, export). Ask the user to allow their folder.",
            output.all_text(),
            unreadable_media.join(", ")
        ));
    }
    if !unreadable_luts.is_empty() {
        output = ToolOutput::text(format!(
            "{}\n\nWarning: the colour LUT files of {} are outside the allowed folders, so Kadr \
             will refuse to show the timeline's pictures or export. Ask the user to allow their \
             folder, or remove those LUTs with remove_effect.",
            output.all_text(),
            unreadable_luts.join(", ")
        ));
    }
    Ok(output)
}

/// The colour LUTs whose file Kadr would refuse to read, as "effect E of clip C".
fn color_luts_outside_allowed_folders(toolbox: &ToolCall<'_>, project: &Project) -> Vec<String> {
    enabled_color_luts(project)
        .filter(|(_, _, color_lut)| {
            check_stored_color_lut_file(toolbox, &color_lut.file_path).is_err()
        })
        .map(|(clip_id, effect_id, _)| format!("effect {effect_id} of clip {clip_id}"))
        .collect()
}

/// The identifiers of the project's media files that lie outside the allowed folders.
fn media_outside_allowed_folders(toolbox: &ToolCall<'_>, project: &Project) -> Vec<String> {
    project
        .media_assets()
        .iter()
        .filter(|asset| {
            toolbox
                .allowed_folders()
                .check_existing_file(&asset.description.file_path)
                .is_err()
        })
        .map(|asset| asset.id.to_string())
        .collect()
}

// ----- save_project -----

const SAVE_PROJECT_DESCRIPTION: &str = "\
Saves the open project. Changes are already saved automatically after every edit, so this is \
mostly useful to save a copy under a new name (\"save as\"): give file_path, and the project \
continues in the new file. Never replaces an existing file unless \"overwrite\": true.";

/// Parameters of `save_project`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SaveProjectParameters {
    /// New full path for the project (".kadr" added if missing). Omit to save to its current file.
    file_path: Option<PathBuf>,
    /// Replace file_path if it already exists. Only true if the user agreed. Default false.
    #[serde(default)]
    overwrite: bool,
}

fn save_project(
    toolbox: &mut ToolCall<'_>,
    parameters: SaveProjectParameters,
) -> Result<ToolOutput, ToolError> {
    let current_path = toolbox.require_project_session()?.file_path().to_path_buf();
    let Some(requested_path) = parameters.file_path else {
        toolbox.require_project_session_mut()?.save()?;
        return Ok(ToolOutput::text(format!(
            "Saved \"{}\".",
            current_path.display()
        )));
    };
    let requested_path = with_extension(&requested_path, "kadr")?;
    let is_current_file = toolbox
        .allowed_folders()
        .check_existing_file(&requested_path)
        .is_ok_and(|canonical| canonical == current_path);
    let new_path = toolbox
        .allowed_folders()
        .check_file_to_write(&requested_path, parameters.overwrite || is_current_file)?;
    toolbox
        .require_project_session_mut()?
        .save_as(new_path.clone())?;
    Ok(ToolOutput::text(format!(
        "Saved the project as \"{}\"; further changes go to this file.",
        new_path.display()
    )))
}

// ----- set_project_settings -----

const SET_PROJECT_SETTINGS_DESCRIPTION: &str = "\
Changes the size of the video (canvas), its frame rate or its background colour. For a vertical \
video (TikTok, Reels, Shorts) use canvas vertical_1080x1920. Clips keep their place relative to \
the centre and still fit inside the new canvas (scale 1); a landscape video in a vertical canvas \
then shows bars above and below: use set_transform with fit \"fill_canvas\" to fill it, and check \
with get_frame. One call is one undo step.

Example: {\"canvas\": \"vertical_1080x1920\"} or {\"width_in_pixels\": 1280, \"height_in_pixels\": \
720, \"frames_per_second\": 25}";

/// Parameters of `set_project_settings`. Omitted values stay unchanged.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SetProjectSettingsParameters {
    /// Standard video size.
    canvas: Option<CanvasPreset>,
    /// Custom canvas width, in pixels (even number from 16 to 8192). Give height_in_pixels too.
    width_in_pixels: Option<u32>,
    /// Custom canvas height, in pixels (even number from 16 to 8192). Give width_in_pixels too.
    height_in_pixels: Option<u32>,
    /// Frames per second. Examples: 24, 25, 30, 60, 29.97.
    frames_per_second: Option<f64>,
    /// Colour where no clip covers the canvas, "#RRGGBB" or "#RRGGBBAA". Example: "#000000".
    background_color: Option<String>,
}

fn set_project_settings(
    toolbox: &mut ToolCall<'_>,
    parameters: SetProjectSettingsParameters,
) -> Result<ToolOutput, ToolError> {
    let current = *toolbox.require_project()?.settings();
    let new_settings = SettingsChanges {
        canvas: parameters.canvas,
        width_in_pixels: parameters.width_in_pixels,
        height_in_pixels: parameters.height_in_pixels,
        frames_per_second: parameters.frames_per_second,
        background_color: parameters.background_color.as_deref(),
    }
    .applied_to(current)?;
    toolbox.change_project("Change the project settings", |transaction| {
        transaction.execute(ChangeProjectSettings::new(new_settings))?;
        Ok(())
    })?;
    Ok(ToolOutput::message_with_details(
        "The project settings are now:",
        &SettingsView::of(&new_settings),
    ))
}

// ----- undo / redo -----

const UNDO_DESCRIPTION: &str = "\
Undoes the last change to the project, whoever made it. Each earlier tool call is one step, so \
one undo reverts one whole tool call (for example a remove_clip with close_gap that moved \
several clips). Call again to go further back. The history lasts while Kadr runs.";

const REDO_DESCRIPTION: &str = "\
Redoes the last change undone by undo (one whole earlier tool call). Only possible right after \
undo: any new change clears what could be redone.";

fn undo(toolbox: &mut ToolCall<'_>, _: NoParameters) -> Result<ToolOutput, ToolError> {
    let label = toolbox.require_project_session_mut()?.undo()?;
    Ok(ToolOutput::text(format!("Undid \"{label}\".")))
}

fn redo(toolbox: &mut ToolCall<'_>, _: NoParameters) -> Result<ToolOutput, ToolError> {
    let label = toolbox.require_project_session_mut()?.redo()?;
    Ok(ToolOutput::text(format!("Redid \"{label}\".")))
}
