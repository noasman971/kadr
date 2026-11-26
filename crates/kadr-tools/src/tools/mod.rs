//! Every tool, grouped by subject. Each submodule declares its tools with their
//! descriptions and parameter types, next to the code that runs them.

mod clip_property_tools;
mod common;
mod content_tools;
mod export_tools;
mod media_tools;
mod picture_tools;
mod project_tools;
mod sound_tools;
mod timeline_tools;

use crate::catalog::ToolEntry;

/// Every tool, in the order they are listed to Claude: project, media, timeline,
/// clip properties, content, pictures, sound, export.
pub(crate) fn all_tools() -> Vec<ToolEntry> {
    let mut tools = project_tools::project_tools();
    tools.extend(media_tools::media_tools());
    tools.extend(timeline_tools::timeline_tools());
    tools.extend(clip_property_tools::clip_property_tools());
    tools.extend(content_tools::content_tools());
    tools.extend(picture_tools::picture_tools());
    tools.extend(sound_tools::sound_tools());
    tools.extend(export_tools::export_tools());
    tools
}
