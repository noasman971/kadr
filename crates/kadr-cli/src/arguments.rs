//! The command-line arguments, declared with `clap`.

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
use kadr_core::text_animation::TextAnimationStyle;
use kadr_core::transition::{SlideDirection, TransitionStyle, ZoomDirection};

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
        /// A transition at the start of every video and image: from the
        /// background for the first one, from the previous clip for the others.
        #[arg(long, value_enum)]
        transition: Option<TransitionChoice>,
        /// How long each transition lasts, in seconds.
        #[arg(long, default_value_t = 0.5)]
        transition_seconds: f64,
        /// A title shown over the start of the video.
        #[arg(long)]
        title: Option<String>,
        /// How long the title stays on screen, in seconds.
        #[arg(long, default_value_t = 3.0)]
        title_seconds: f64,
        /// How the title appears and disappears.
        #[arg(long, value_enum, default_value_t = TitleAnimationChoice::TypeOn)]
        title_animation: TitleAnimationChoice,
        /// How long the title takes to appear (and to disappear), in seconds.
        #[arg(long, default_value_t = 0.8)]
        title_animation_seconds: f64,
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
        /// Render on the CPU even if a GPU is available.
        #[arg(long)]
        cpu: bool,
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

/// The transitions `assemble --transition` offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum TransitionChoice {
    /// Fade from black, or dissolve from the previous clip.
    Fade,
    /// Come in from the right, pushing the previous clip to the left.
    SlideLeft,
    /// Come in from the left, pushing the previous clip to the right.
    SlideRight,
    /// Come in from the bottom, pushing the previous clip up.
    SlideUp,
    /// Come in from the top, pushing the previous clip down.
    SlideDown,
    /// Grow into place while the previous clip grows away.
    ZoomIn,
    /// Shrink into place while the previous clip shrinks away.
    ZoomOut,
}

impl TransitionChoice {
    /// The transition style it stands for.
    pub fn style(self) -> TransitionStyle {
        let slide = |direction| TransitionStyle::Slide { direction };
        let zoom = |direction| TransitionStyle::Zoom { direction };
        match self {
            TransitionChoice::Fade => TransitionStyle::Fade,
            TransitionChoice::SlideLeft => slide(SlideDirection::Left),
            TransitionChoice::SlideRight => slide(SlideDirection::Right),
            TransitionChoice::SlideUp => slide(SlideDirection::Up),
            TransitionChoice::SlideDown => slide(SlideDirection::Down),
            TransitionChoice::ZoomIn => zoom(ZoomDirection::In),
            TransitionChoice::ZoomOut => zoom(ZoomDirection::Out),
        }
    }
}

/// The text animations `assemble --title-animation` offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum TitleAnimationChoice {
    /// Fade in and out.
    Fade,
    /// Rise into place, and on out of the picture.
    SlideUp,
    /// Grow from nothing with a small bounce.
    Pop,
    /// Letters appear one after another, and are erased at the end.
    TypeOn,
    /// Words appear one after another.
    WordByWord,
}

impl TitleAnimationChoice {
    /// The text animation style it stands for.
    pub fn style(self) -> TextAnimationStyle {
        match self {
            TitleAnimationChoice::Fade => TextAnimationStyle::Fade,
            TitleAnimationChoice::SlideUp => TextAnimationStyle::Slide {
                direction: SlideDirection::Up,
            },
            TitleAnimationChoice::Pop => TextAnimationStyle::Pop,
            TitleAnimationChoice::TypeOn => TextAnimationStyle::TypeOn,
            TitleAnimationChoice::WordByWord => TextAnimationStyle::WordByWord,
        }
    }
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
            cpu,
        } = parsed.command
        else {
            panic!("expected the render command");
        };
        assert_eq!(project, PathBuf::from("edit.kadr"));
        assert_eq!(output, PathBuf::from("out.mp4"));
        assert!(fast && !software_encoder && !cpu);
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
                transition: None,
                title: None,
                ..
            }
        ));
    }

    #[test]
    fn assemble_takes_a_transition_and_an_animated_title() {
        let parsed = CommandLine::try_parse_from([
            "kadr-cli",
            "assemble",
            "-o",
            "edit.kadr",
            "--transition",
            "slide-left",
            "--title",
            "Hello",
            "--title-animation",
            "word-by-word",
            "a.mp4",
        ])
        .unwrap();
        let Command::Assemble {
            transition,
            transition_seconds,
            title,
            title_animation,
            ..
        } = parsed.command
        else {
            panic!("expected the assemble command");
        };
        assert_eq!(transition, Some(TransitionChoice::SlideLeft));
        assert_eq!(transition_seconds, 0.5);
        assert_eq!(title.as_deref(), Some("Hello"));
        assert_eq!(title_animation.style(), TextAnimationStyle::WordByWord);
    }
}
