//! The `remove-silences` command: shorten the silences of a file to short
//! pauses and export the result.
//!
//! 1. A project is built around the file (a canvas of the video's size).
//! 2. The silences of its sound are found and shortened
//!    (`kadr_engine::silence_removal`), in one undoable change.
//! 3. The project is exported (and saved with `--save-project`).

use std::error::Error;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use kadr_core::history::EditSession;
use kadr_core::project::ProjectSettings;
use kadr_core::time::TimeSpan;
use kadr_engine::project_files::save_project_file;
use kadr_engine::quick_assembly::assemble_project_from_files;
use kadr_engine::silence_removal::{SilenceRemovalSettings, plan_silence_removal_with_progress};
use kadr_engine::{ExportCancellation, ExportRequest, RendererPreference, export_project};
use kadr_media::{EncodingSpeed, FfmpegBackend, MediaBackend, VideoEncoderPreference};

use crate::blur::even_size;
use crate::reports;

/// Still images have no sound, but the assembly needs a duration for them.
const STILL_IMAGE_DURATION: TimeSpan = TimeSpan::from_whole_seconds(5);

/// Everything the `remove-silences` command was asked to do.
#[derive(Debug, Clone)]
pub struct SilenceRemovalCommand {
    /// The video or sound file.
    pub file: PathBuf,
    /// The MP4 file to create.
    pub output: PathBuf,
    /// What counts as a silence, and how much of it stays.
    pub settings: SilenceRemovalSettings,
    /// Where to save the project, if asked.
    pub project_output: Option<PathBuf>,
    /// Never use a hardware encoder.
    pub software_encoder: bool,
    /// Encode faster.
    pub fast: bool,
    /// Render on the CPU.
    pub cpu: bool,
}

/// Runs the `remove-silences` command.
pub fn remove_silences(
    backend: &FfmpegBackend,
    command: &SilenceRemovalCommand,
) -> Result<(), Box<dyn Error>> {
    refuse_output_replacing_the_file(command)?;
    let description = backend.probe(&command.file)?;
    let defaults = ProjectSettings::landscape_1080p();
    let settings = ProjectSettings {
        canvas_size: description
            .picture_size
            .map_or(defaults.canvas_size, even_size),
        frame_rate: description.frame_rate.unwrap_or(defaults.frame_rate),
        ..defaults
    };
    let project = assemble_project_from_files(
        backend,
        std::slice::from_ref(&command.file),
        settings,
        STILL_IMAGE_DURATION,
    )?;
    let clip_id = *project
        .timeline()
        .clip_ids()
        .first()
        .ok_or("the file could not be placed on the timeline")?;

    let analysis_stopped = Arc::new(AtomicBool::new(false));
    let export_cancellation = ExportCancellation::new();
    {
        let (analysis, export) = (Arc::clone(&analysis_stopped), export_cancellation.clone());
        // Ctrl+C stops cleanly instead of leaving a half-written file.
        ctrlc::set_handler(move || {
            analysis.store(true, Ordering::Relaxed);
            export.cancel();
        })?;
    }

    let mut progress_line = reports::ProgressLine::default();
    let plan = plan_silence_removal_with_progress(
        &project,
        backend,
        clip_id,
        command.settings,
        |fraction| {
            progress_line.update_fraction("Looking for silences", fraction);
            !analysis_stopped.load(Ordering::Relaxed)
        },
    );
    progress_line.finish();
    let plan = plan?;
    let duration_before = project.timeline().duration();
    if plan.whole_clip_is_silent {
        return Err(format!(
            "{} is silent from start to end: nothing would be left",
            command.file.display()
        )
        .into());
    }
    let mut session = EditSession::new(project)?;
    match plan.removal_command() {
        Some(removal) => {
            session.execute(removal)?;
            println!(
                "Shortened {} silence(s): {} removed, {} → {}",
                plan.cuts.len(),
                plan.removed_duration(),
                duration_before,
                session.project().timeline().duration()
            );
        }
        None => println!("No silence long enough to shorten: the file is exported as it is"),
    }
    if let Some(project_path) = &command.project_output {
        save_project_file(session.project(), project_path)?;
        println!("Saved {}", project_path.display());
    }

    let export_request = ExportRequest {
        output_path: command.output.clone(),
        video_encoder_preference: if command.software_encoder {
            VideoEncoderPreference::SoftwareOnly
        } else {
            VideoEncoderPreference::HardwareFirst
        },
        encoding_speed: if command.fast {
            EncodingSpeed::Fast
        } else {
            EncodingSpeed::Balanced
        },
        renderer_preference: if command.cpu {
            RendererPreference::CpuOnly
        } else {
            RendererPreference::GpuIfAvailable
        },
    };
    let started_at = Instant::now();
    let mut progress_line = reports::ProgressLine::default();
    let exported = export_project(
        session.project(),
        backend,
        &export_request,
        &mut |progress| progress_line.update(progress),
        &export_cancellation,
    );
    progress_line.finish();
    println!(
        "{}",
        reports::describe_export(&exported?, started_at.elapsed())
    );
    Ok(())
}

/// Refuses an output that would replace the file being shortened.
fn refuse_output_replacing_the_file(command: &SilenceRemovalCommand) -> Result<(), Box<dyn Error>> {
    let file = std::fs::canonicalize(&command.file)?;
    let outputs = std::iter::once(&command.output).chain(command.project_output.as_ref());
    for output in outputs {
        // A file that does not exist yet cannot be the source.
        if std::fs::canonicalize(output).is_ok_and(|path| path == file) {
            return Err(format!(
                "{} is the file whose silences are removed: choose another output",
                output.display()
            )
            .into());
        }
    }
    Ok(())
}
