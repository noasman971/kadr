//! Editing sessions: applying commands, with undo and redo.
//!
//! An [`EditSession`] owns a project and is the only way to change it. Every
//! change is checked by [`validate_project`]: a change that would break a rule
//! is undone immediately and refused, so the project is **always valid**.
//!
//! Several commands can be grouped into one undo step with
//! [`EditSession::transaction`]: a request from Claude that changes ten things
//! is undone with a single Ctrl+Z, and if any step fails, the steps already done
//! are undone and nothing changes.
//!
//! # Example
//!
//! ```
//! use kadr_core::commands::{AddClip, AddTrack, CreatedObjectsExt, TrackPosition};
//! use kadr_core::history::EditSession;
//! use kadr_core::project::{Project, ProjectSettings};
//! use kadr_core::time::{TimeSpan, Timestamp};
//! use kadr_core::timeline::TrackKind;
//!
//! let project = Project::new(ProjectSettings::vertical_1080x1920());
//! let mut session = EditSession::new(project).unwrap();
//!
//! session
//!     .transaction("Add a title", |transaction| {
//!         let created = transaction.execute(AddTrack::new(TrackKind::Video, TrackPosition::Top))?;
//!         let track_id = created.first_track_id().expect("a track was created");
//!         transaction.execute(AddClip::text(
//!             track_id,
//!             "Hello",
//!             TimeSpan::from_whole_seconds(3),
//!             Timestamp::ZERO,
//!         ))?;
//!         Ok(())
//!     })
//!     .unwrap();
//!
//! assert_eq!(session.project().timeline().clip_ids().len(), 1);
//! session.undo().unwrap();
//! assert!(session.project().timeline().tracks().is_empty());
//! ```

use crate::commands::{CreatedObject, EditCommand};
use crate::error::EditError;
use crate::project::Project;
use crate::validation::{ValidationError, validate_project};

/// One step of the undo history: one or more commands applied together.
#[derive(Debug)]
struct HistoryStep {
    label: String,
    commands_in_application_order: Vec<Box<dyn EditCommand>>,
}

/// A project being edited, with its undo and redo history.
#[derive(Debug)]
pub struct EditSession {
    project: Project,
    undo_steps: Vec<HistoryStep>,
    redo_steps: Vec<HistoryStep>,
    maximum_undo_steps: usize,
}

impl EditSession {
    /// How many steps can be undone by default. Older steps are forgotten.
    pub const DEFAULT_MAXIMUM_UNDO_STEPS: usize = 500;

    /// Starts editing a project.
    ///
    /// # Errors
    ///
    /// Fails if the project breaks a rule, so that editing never starts from an
    /// invalid state.
    pub fn new(project: Project) -> Result<Self, ValidationError> {
        validate_project(&project)?;
        Ok(Self {
            project,
            undo_steps: Vec::new(),
            redo_steps: Vec::new(),
            maximum_undo_steps: Self::DEFAULT_MAXIMUM_UNDO_STEPS,
        })
    }

    /// Changes how many steps can be undone (at least 1).
    pub fn with_maximum_undo_steps(mut self, maximum_undo_steps: usize) -> Self {
        self.maximum_undo_steps = maximum_undo_steps.max(1);
        self.forget_oldest_steps_beyond_limit();
        self
    }

    /// The project, in its current state.
    pub fn project(&self) -> &Project {
        &self.project
    }

    /// Stops editing and gives the project back.
    pub fn into_project(self) -> Project {
        self.project
    }

    /// Applies one command as one undo step.
    ///
    /// # Errors
    ///
    /// Fails if the command is impossible or would make the project invalid.
    /// The project is then unchanged.
    pub fn execute(
        &mut self,
        command: impl EditCommand + 'static,
    ) -> Result<Vec<CreatedObject>, EditError> {
        let label = command.label();
        self.transaction(label, |transaction| transaction.execute(command))
    }

    /// Runs several commands as **one** undo step.
    ///
    /// `steps` receives a [`Transaction`] to execute commands one after the
    /// other; it can read the project and use the identifiers created by earlier
    /// steps. If `steps` returns an error, every command it applied is reverted
    /// and the error is returned: all or nothing.
    ///
    /// # Errors
    ///
    /// Returns the error of the failing step.
    pub fn transaction<Output>(
        &mut self,
        label: impl Into<String>,
        steps: impl FnOnce(&mut Transaction<'_>) -> Result<Output, EditError>,
    ) -> Result<Output, EditError> {
        let mut transaction = Transaction {
            project: &mut self.project,
            applied_commands: Vec::new(),
        };
        match steps(&mut transaction) {
            Ok(output) => {
                let applied_commands = transaction.applied_commands;
                if !applied_commands.is_empty() {
                    self.record_step(HistoryStep {
                        label: label.into(),
                        commands_in_application_order: applied_commands,
                    });
                }
                Ok(output)
            }
            Err(step_error) => {
                transaction.revert_everything()?;
                Err(step_error)
            }
        }
    }

    /// Runs a list of commands as one undo step. If one fails, the error says
    /// which step it was, and nothing changes.
    ///
    /// # Errors
    ///
    /// [`EditError::StepFailed`] naming the failing step.
    pub fn execute_all(
        &mut self,
        label: impl Into<String>,
        commands: Vec<Box<dyn EditCommand>>,
    ) -> Result<Vec<CreatedObject>, EditError> {
        self.transaction(label, |transaction| {
            let mut all_created_objects = Vec::new();
            for (index, command) in commands.into_iter().enumerate() {
                let step_label = command.label();
                let created_objects =
                    transaction
                        .execute_boxed(command)
                        .map_err(|cause| EditError::StepFailed {
                            step_number: index + 1,
                            step_label,
                            cause: Box::new(cause),
                        })?;
                all_created_objects.extend(created_objects);
            }
            Ok(all_created_objects)
        })
    }

    /// Undoes the most recent step and returns its label.
    ///
    /// # Errors
    ///
    /// [`EditError::NothingToUndo`] when the history is empty.
    pub fn undo(&mut self) -> Result<String, EditError> {
        let mut step = self.undo_steps.pop().ok_or(EditError::NothingToUndo)?;
        for command in step.commands_in_application_order.iter_mut().rev() {
            command.revert(&mut self.project)?;
        }
        ensure_still_valid(&self.project, "undo")?;
        let label = step.label.clone();
        self.redo_steps.push(step);
        Ok(label)
    }

    /// Redoes the most recently undone step and returns its label.
    ///
    /// # Errors
    ///
    /// [`EditError::NothingToRedo`] when nothing was undone since the last change.
    pub fn redo(&mut self) -> Result<String, EditError> {
        let mut step = self.redo_steps.pop().ok_or(EditError::NothingToRedo)?;
        for command in &mut step.commands_in_application_order {
            command.apply(&mut self.project)?;
        }
        ensure_still_valid(&self.project, "redo")?;
        let label = step.label.clone();
        self.undo_steps.push(step);
        Ok(label)
    }

    /// Whether there is something to undo.
    pub fn can_undo(&self) -> bool {
        !self.undo_steps.is_empty()
    }

    /// Whether there is something to redo.
    pub fn can_redo(&self) -> bool {
        !self.redo_steps.is_empty()
    }

    /// The label of the step [`EditSession::undo`] would undo.
    pub fn next_undo_label(&self) -> Option<&str> {
        self.undo_steps.last().map(|step| step.label.as_str())
    }

    /// The label of the step [`EditSession::redo`] would redo.
    pub fn next_redo_label(&self) -> Option<&str> {
        self.redo_steps.last().map(|step| step.label.as_str())
    }

    /// The labels of every step that can be undone, oldest first.
    pub fn undo_history_labels(&self) -> Vec<&str> {
        self.undo_steps
            .iter()
            .map(|step| step.label.as_str())
            .collect()
    }

    /// Records a new step: it becomes the next to undo, and redo is no longer possible.
    fn record_step(&mut self, step: HistoryStep) {
        self.undo_steps.push(step);
        self.redo_steps.clear();
        self.forget_oldest_steps_beyond_limit();
    }

    fn forget_oldest_steps_beyond_limit(&mut self) {
        let excess_step_count = self
            .undo_steps
            .len()
            .saturating_sub(self.maximum_undo_steps);
        self.undo_steps.drain(..excess_step_count);
    }
}

/// Undo and redo replay commands that were valid before; if the result is
/// invalid, the history itself is broken, which is a bug.
fn ensure_still_valid(project: &Project, operation_name: &str) -> Result<(), EditError> {
    validate_project(project).map_err(|validation_error| EditError::InternalInconsistency {
        details: format!("{operation_name} produced an invalid project: {validation_error}"),
    })
}

/// Commands being applied as one undo step. See [`EditSession::transaction`].
pub struct Transaction<'session> {
    project: &'session mut Project,
    applied_commands: Vec<Box<dyn EditCommand>>,
}

impl Transaction<'_> {
    /// The project, including the changes of the steps already executed.
    pub fn project(&self) -> &Project {
        self.project
    }

    /// Applies a command and checks the project is still valid.
    ///
    /// # Errors
    ///
    /// Fails if the command is impossible or would make the project invalid; the
    /// project is then as it was before this command.
    pub fn execute(
        &mut self,
        command: impl EditCommand + 'static,
    ) -> Result<Vec<CreatedObject>, EditError> {
        self.execute_boxed(Box::new(command))
    }

    /// Same as [`Transaction::execute`], for a command already boxed.
    ///
    /// # Errors
    ///
    /// See [`Transaction::execute`].
    pub fn execute_boxed(
        &mut self,
        mut command: Box<dyn EditCommand>,
    ) -> Result<Vec<CreatedObject>, EditError> {
        let created_objects = command.apply(self.project)?;
        if let Err(validation_error) = validate_project(self.project) {
            command.revert(self.project)?;
            return Err(EditError::WouldMakeProjectInvalid(validation_error));
        }
        self.applied_commands.push(command);
        Ok(created_objects)
    }

    /// Reverts every applied command, most recent first.
    fn revert_everything(&mut self) -> Result<(), EditError> {
        while let Some(mut command) = self.applied_commands.pop() {
            command.revert(self.project)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip::AnimatableProperty;
    use crate::commands::{
        AddClip, AddTrack, CreatedObjectsExt, ImportMedia, MoveClip, SetClipPropertyValue,
        SplitClip, TrackPosition,
    };
    use crate::ids::{ClipId, TrackId};
    use crate::media_asset::test_samples::ten_second_video;
    use crate::project::ProjectSettings;
    use crate::time::{TimeSpan, Timestamp};
    use crate::timeline::TrackKind;

    fn seconds(value: u64) -> Timestamp {
        Timestamp::from_whole_seconds(value)
    }

    fn session_with_track() -> (EditSession, TrackId) {
        let mut session =
            EditSession::new(Project::new(ProjectSettings::landscape_1080p())).unwrap();
        let track_id = session
            .execute(AddTrack::new(TrackKind::Video, TrackPosition::Top))
            .unwrap()
            .first_track_id()
            .unwrap();
        (session, track_id)
    }

    fn add_title(session: &mut EditSession, track_id: TrackId, start_second: u64) -> ClipId {
        session
            .execute(AddClip::text(
                track_id,
                "Title",
                TimeSpan::from_whole_seconds(2),
                seconds(start_second),
            ))
            .unwrap()
            .first_clip_id()
            .unwrap()
    }

    #[test]
    fn undo_and_redo_walk_through_the_history() {
        let (mut session, track_id) = session_with_track();
        let project_before_clip = session.project().clone();
        let clip_id = add_title(&mut session, track_id, 0);
        let project_with_clip = session.project().clone();

        assert_eq!(
            session.next_undo_label(),
            Some("Add text to track 1 at 0.000 s")
        );
        session.undo().unwrap();
        assert_eq!(session.project(), &project_before_clip);
        assert!(session.can_redo());

        session.redo().unwrap();
        assert_eq!(session.project(), &project_with_clip);
        assert!(session.project().timeline().find_clip(clip_id).is_some());
    }

    #[test]
    fn a_new_change_clears_the_redo_history() {
        let (mut session, track_id) = session_with_track();
        add_title(&mut session, track_id, 0);
        session.undo().unwrap();
        add_title(&mut session, track_id, 5);
        assert!(!session.can_redo());
        assert_eq!(session.redo(), Err(EditError::NothingToRedo));
    }

    #[test]
    fn a_change_that_breaks_a_rule_is_refused_and_leaves_nothing() {
        let (mut session, track_id) = session_with_track();
        let first_clip_id = add_title(&mut session, track_id, 0);
        add_title(&mut session, track_id, 5);
        let project_before = session.project().clone();
        let history_before = session.undo_history_labels().len();

        let overlap = session.execute(MoveClip::along_track(first_clip_id, seconds(6)));
        assert!(matches!(
            overlap,
            Err(EditError::WouldMakeProjectInvalid(
                ValidationError::ClipsOverlap { .. }
            ))
        ));

        let out_of_range = session.execute(SetClipPropertyValue::new(
            first_clip_id,
            AnimatableProperty::Opacity,
            2.0,
        ));
        assert!(
            out_of_range
                .unwrap_err()
                .to_string()
                .contains("must be between 0 and 1")
        );

        assert_eq!(session.project(), &project_before);
        assert_eq!(session.undo_history_labels().len(), history_before);
    }

    #[test]
    fn a_transaction_is_one_undo_step_and_all_or_nothing() {
        let (mut session, track_id) = session_with_track();
        let project_before = session.project().clone();

        session
            .transaction("Two titles", |transaction| {
                transaction.execute(AddClip::text(
                    track_id,
                    "A",
                    TimeSpan::from_whole_seconds(1),
                    seconds(0),
                ))?;
                transaction.execute(AddClip::text(
                    track_id,
                    "B",
                    TimeSpan::from_whole_seconds(1),
                    seconds(1),
                ))?;
                Ok(())
            })
            .unwrap();
        assert_eq!(session.project().timeline().clip_ids().len(), 2);
        assert_eq!(session.undo().unwrap(), "Two titles");
        assert_eq!(session.project(), &project_before);

        let failed = session.transaction("Doomed", |transaction| {
            transaction.execute(AddClip::text(
                track_id,
                "A",
                TimeSpan::from_whole_seconds(1),
                seconds(0),
            ))?;
            transaction.execute(AddClip::text(
                track_id,
                "Overlaps",
                TimeSpan::from_whole_seconds(1),
                seconds(0),
            ))?;
            Ok(())
        });
        assert!(failed.is_err());
        assert_eq!(session.project(), &project_before);
        assert_eq!(session.next_undo_label(), Some("Add video track"));
    }

    #[test]
    fn execute_all_names_the_failing_step() {
        let (mut session, track_id) = session_with_track();
        let commands: Vec<Box<dyn EditCommand>> = vec![
            Box::new(AddClip::text(
                track_id,
                "A",
                TimeSpan::from_whole_seconds(2),
                seconds(0),
            )),
            Box::new(SplitClip::new(ClipId::from_raw(99), seconds(1))),
        ];
        let error = session.execute_all("Batch", commands).unwrap_err();
        assert!(matches!(
            error,
            EditError::StepFailed { step_number: 2, .. }
        ));
        assert!(
            error
                .to_string()
                .starts_with("step 2 (Split clip 99 at 1.000 s) failed")
        );
        assert!(session.project().timeline().clip_ids().is_empty());
    }

    #[test]
    fn redo_gives_back_the_same_identifiers() {
        let mut session =
            EditSession::new(Project::new(ProjectSettings::landscape_1080p())).unwrap();
        let first_media_id = session
            .execute(ImportMedia::new(ten_second_video()))
            .unwrap()
            .first_media_asset_id();
        session.undo().unwrap();
        session.redo().unwrap();
        assert_eq!(
            session.project().media_assets()[0].id,
            first_media_id.unwrap()
        );
    }

    #[test]
    fn the_oldest_steps_are_forgotten_beyond_the_limit() {
        let (session, track_id) = session_with_track();
        let mut session = session.with_maximum_undo_steps(2);
        for start_second in [0, 3, 6] {
            add_title(&mut session, track_id, start_second);
        }
        assert_eq!(session.undo_history_labels().len(), 2);
        session.undo().unwrap();
        session.undo().unwrap();
        assert_eq!(session.undo(), Err(EditError::NothingToUndo));
        assert_eq!(session.project().timeline().clip_ids().len(), 1);
    }

    #[test]
    fn an_invalid_project_cannot_start_a_session() {
        let mut settings = ProjectSettings::landscape_1080p();
        settings.canvas_size.width_in_pixels = 1921;
        let error = EditSession::new(Project::new(settings)).unwrap_err();
        assert!(error.to_string().contains("even number of pixels"));
    }
}
