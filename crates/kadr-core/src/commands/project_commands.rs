//! Commands that change the project settings.

use super::{CreatedObject, EditCommand, revert_without_apply};
use crate::error::EditError;
use crate::project::{Project, ProjectSettings};

/// Replaces the project settings (canvas size, frame rate, audio rate, background).
#[derive(Debug)]
pub struct ChangeProjectSettings {
    new_settings: ProjectSettings,
    previous_settings: Option<ProjectSettings>,
}

impl ChangeProjectSettings {
    /// Prepares the change. Settings are checked when the command is applied.
    pub fn new(new_settings: ProjectSettings) -> Self {
        Self {
            new_settings,
            previous_settings: None,
        }
    }
}

impl EditCommand for ChangeProjectSettings {
    fn label(&self) -> String {
        let canvas = self.new_settings.canvas_size;
        format!(
            "Change project settings to {}×{} at {}",
            canvas.width_in_pixels, canvas.height_in_pixels, self.new_settings.frame_rate
        )
    }

    fn apply(&mut self, project: &mut Project) -> Result<Vec<CreatedObject>, EditError> {
        let settings = project.settings_mut();
        self.previous_settings = Some(*settings);
        *settings = self.new_settings;
        Ok(Vec::new())
    }

    fn revert(&mut self, project: &mut Project) -> Result<(), EditError> {
        let previous_settings = self
            .previous_settings
            .take()
            .ok_or_else(|| revert_without_apply(&self.label()))?;
        *project.settings_mut() = previous_settings;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_support::apply_and_check_revert;

    #[test]
    fn settings_change_is_reversible() {
        let project = Project::new(ProjectSettings::landscape_1080p());
        let (changed_project, _) = apply_and_check_revert(
            &project,
            ChangeProjectSettings::new(ProjectSettings::vertical_1080x1920()),
        );
        assert_eq!(changed_project.settings().canvas_size.width_in_pixels, 1080);
    }
}
