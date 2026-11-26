//! The command-line arguments, declared with `clap`.

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

/// Kadr from the command line: inspect media files, build a project, export it.
#[derive(Debug, Parser)]
#[command(name = "kadr-cli", version, about)]
pub struct CommandLine {
    /// What to do.
    #[command(subcommand)]
    pub command: Command,
}

/// The available commands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Describe a media file: kind, duration, picture size, frame rate, sound.
    Probe {
        /// The video, audio or image file.
        file: PathBuf,
        /// Print the description as JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Create a project placing files end to end (videos and images on a video
    /// track, sound files on an audio track).
    Assemble {
        /// The `.kadr` project file to create (replaced if it exists).
        #[arg(long, short)]
        output: PathBuf,
        /// The shape of the video.
        #[arg(long, value_enum, default_value_t = CanvasFormat::Landscape)]
        format: CanvasFormat,
        /// How long each still image is shown, in seconds.
        #[arg(long, default_value_t = 5.0)]
        image_duration: f64,
        /// The media files, in order.
        #[arg(required = true)]
        files: Vec<PathBuf>,
    },
    /// Export a project to an MP4 video (H.264 + AAC).
    Render {
        /// The `.kadr` project file.
        project: PathBuf,
        /// The MP4 file to create (replaced only if the export succeeds).
        output: PathBuf,
        /// Never use a hardware encoder.
        #[arg(long)]
        software_encoder: bool,
        /// Encode faster, with larger files (for previews).
        #[arg(long)]
        fast: bool,
    },
}

/// The shape of the video created by `assemble`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum CanvasFormat {
    /// 1920 × 1080 (YouTube).
    Landscape,
    /// 1080 × 1920 (TikTok, Reels, Shorts).
    Vertical,
    /// 1080 × 1080.
    Square,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn the_argument_definitions_are_consistent() {
        CommandLine::command().debug_assert();
    }

    #[test]
    fn render_options_are_parsed() {
        let parsed =
            CommandLine::try_parse_from(["kadr-cli", "render", "edit.kadr", "out.mp4", "--fast"])
                .unwrap();
        let Command::Render {
            project,
            output,
            software_encoder,
            fast,
        } = parsed.command
        else {
            panic!("expected the render command");
        };
        assert_eq!(project, PathBuf::from("edit.kadr"));
        assert_eq!(output, PathBuf::from("out.mp4"));
        assert!(fast && !software_encoder);
    }

    #[test]
    fn assemble_needs_at_least_one_file() {
        assert!(CommandLine::try_parse_from(["kadr-cli", "assemble", "-o", "edit.kadr"]).is_err());
        let parsed = CommandLine::try_parse_from([
            "kadr-cli",
            "assemble",
            "-o",
            "edit.kadr",
            "--format",
            "vertical",
            "a.mp4",
        ])
        .unwrap();
        assert!(matches!(
            parsed.command,
            Command::Assemble {
                format: CanvasFormat::Vertical,
                ..
            }
        ));
    }
}
