//! # kadr-cli — Kadr from the command line
//!
//! A small binary to use the engine without the desktop app:
//!
//! ```text
//! kadr-cli probe video.mp4
//! kadr-cli assemble --output edit.kadr --format vertical clip1.mp4 clip2.mp4 music.mp3
//! kadr-cli assemble --output demo.kadr --transition fade --title "Hello" clip1.mp4 clip2.mp4
//! kadr-cli render edit.kadr edit.mp4
//! kadr-cli blur interview.mp4 --at 2.5 --rect 420,180,160,200 -o blurred.mp4 --debug
//! ```

mod arguments;
mod blur;
mod finishing_touches;
mod reports;

use std::error::Error;
use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

use clap::Parser;
use kadr_core::project::{Project, ProjectSettings};
use kadr_core::text_animation::TextAnimation;
use kadr_core::time::TimeSpan;
use kadr_core::transition::Transition;
use kadr_engine::project_files::{load_project_file, save_project_file};
use kadr_engine::quick_assembly::assemble_project_from_files;
use kadr_engine::{ExportCancellation, ExportRequest, RendererPreference, export_project};
use kadr_media::{EncodingSpeed, FfmpegBackend, MediaBackend, VideoEncoderPreference};

use arguments::{CanvasFormat, Command, CommandLine, TitleAnimationChoice, TransitionChoice};

fn main() -> ExitCode {
    let command_line = CommandLine::parse();
    match run(command_line.command) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(command: Command) -> Result<(), Box<dyn Error>> {
    let backend = FfmpegBackend::new()?;
    match command {
        Command::Probe { file, json } => probe(&backend, &file, json),
        Command::Assemble {
            output,
            format,
            image_duration,
            transition,
            transition_seconds,
            title,
            title_seconds,
            title_animation,
            title_animation_seconds,
            files,
        } => {
            let image_duration = TimeSpan::from_seconds(image_duration)?;
            let project = assemble_project_from_files(
                &backend,
                &files,
                settings_for(format),
                image_duration,
            )?;
            let project = add_finishing_touches(
                project,
                FinishingTouches {
                    transition: transition.map(|choice| (choice, transition_seconds)),
                    title: title.map(|text| (text, title_seconds)),
                    title_animation: (title_animation, title_animation_seconds),
                },
            )?;
            save_project_file(&project, &output)?;
            println!("{}", reports::describe_assembled_project(&output, &project));
            Ok(())
        }
        Command::Render {
            project,
            output,
            software_encoder,
            fast,
            cpu,
        } => render(
            &backend,
            &project,
            &output,
            RenderOptions {
                software_encoder,
                fast,
                cpu,
            },
        ),
        Command::Blur {
            video,
            at,
            rect,
            output,
            style,
            debug,
            save_project,
            software_encoder,
            fast,
            cpu,
        } => blur::blur(
            &backend,
            &blur::BlurCommand {
                video,
                at_seconds: at,
                rectangle: rect,
                output,
                style,
                write_debug_video: debug,
                project_output: save_project,
                software_encoder,
                fast,
                cpu,
            },
        ),
    }
}

/// The optional `assemble` switches, with their durations in seconds.
struct FinishingTouches {
    transition: Option<(TransitionChoice, f64)>,
    title: Option<(String, f64)>,
    title_animation: (TitleAnimationChoice, f64),
}

fn add_finishing_touches(
    mut project: Project,
    touches: FinishingTouches,
) -> Result<Project, Box<dyn Error>> {
    if let Some((choice, duration_in_seconds)) = touches.transition {
        let transition = Transition {
            style: choice.style(),
            duration: TimeSpan::from_seconds(duration_in_seconds)?,
        };
        project = finishing_touches::add_transitions_at_every_cut(project, transition)?;
    }
    if let Some((text, duration_in_seconds)) = touches.title {
        let (animation_choice, animation_seconds) = touches.title_animation;
        let animation = TextAnimation {
            style: animation_choice.style(),
            duration: TimeSpan::from_seconds(animation_seconds)?,
        };
        project = finishing_touches::add_animated_title(
            project,
            &text,
            TimeSpan::from_seconds(duration_in_seconds)?,
            animation,
        )?;
    }
    Ok(project)
}

fn settings_for(format: CanvasFormat) -> ProjectSettings {
    match format {
        CanvasFormat::Landscape => ProjectSettings::landscape_1080p(),
        CanvasFormat::Vertical => ProjectSettings::vertical_1080x1920(),
        CanvasFormat::Square => ProjectSettings::square_1080(),
    }
}

fn probe(backend: &FfmpegBackend, file: &Path, as_json: bool) -> Result<(), Box<dyn Error>> {
    let description = backend.probe(file)?;
    if as_json {
        println!("{}", serde_json::to_string_pretty(&description)?);
    } else {
        println!("{}", reports::describe_media(&description));
    }
    Ok(())
}

/// The `render` switches.
struct RenderOptions {
    software_encoder: bool,
    fast: bool,
    cpu: bool,
}

fn render(
    backend: &FfmpegBackend,
    project_path: &Path,
    output_path: &Path,
    options: RenderOptions,
) -> Result<(), Box<dyn Error>> {
    let project = load_project_file(project_path)?;
    let cancellation = ExportCancellation::new();
    let cancellation_for_ctrl_c = cancellation.clone();
    // Ctrl+C stops the export cleanly instead of leaving a half-written file.
    ctrlc::set_handler(move || cancellation_for_ctrl_c.cancel())?;

    let request = ExportRequest {
        output_path: output_path.to_path_buf(),
        video_encoder_preference: if options.software_encoder {
            VideoEncoderPreference::SoftwareOnly
        } else {
            VideoEncoderPreference::HardwareFirst
        },
        encoding_speed: if options.fast {
            EncodingSpeed::Fast
        } else {
            EncodingSpeed::Balanced
        },
        renderer_preference: if options.cpu {
            RendererPreference::CpuOnly
        } else {
            RendererPreference::GpuIfAvailable
        },
    };
    let started_at = Instant::now();
    let mut progress_line = reports::ProgressLine::default();
    let report = export_project(
        &project,
        backend,
        &request,
        &mut |progress| progress_line.update(progress),
        &cancellation,
    );
    progress_line.finish();
    println!(
        "{}",
        reports::describe_export(&report?, started_at.elapsed())
    );
    Ok(())
}
