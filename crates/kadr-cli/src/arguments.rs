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
        /// Render on the CPU even if a GPU is available.
        #[arg(long)]
        cpu: bool,
    },
    /// Blur a region of a video through the whole video: the region selected at
    /// one moment is followed forwards and backwards by the tracker (no AI).
    Blur {
        /// The video file.
        video: PathBuf,
        /// The moment the region is selected, in seconds from the start.
        #[arg(long)]
        at: f64,
        /// The region at that moment, in the video's pixels: `left,top,width,height`
        /// (origin at the top-left corner), for example `420,180,160,200`.
        #[arg(long, value_parser = parse_rectangle, allow_hyphen_values = true)]
        rect: RectangleArgument,
        /// The MP4 file to create.
        #[arg(long, short)]
        output: PathBuf,
        /// How the region is hidden.
        #[arg(long, value_enum, default_value_t = HidingStyle::Blur)]
        style: HidingStyle,
        /// Also write `<output>.debug.mp4`, showing the tracked box and the
        /// tracker's confidence on every frame.
        #[arg(long)]
        debug: bool,
        /// Also save the project, with the tracked blur, to this `.kadr` file.
        #[arg(long)]
        save_project: Option<PathBuf>,
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

/// A rectangle given on the command line as `left,top,width,height`, in pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RectangleArgument {
    /// Distance from the left edge.
    pub left: f32,
    /// Distance from the top edge.
    pub top: f32,
    /// Width.
    pub width: f32,
    /// Height.
    pub height: f32,
}

/// Reads `left,top,width,height`.
fn parse_rectangle(text: &str) -> Result<RectangleArgument, String> {
    let numbers: Vec<f32> = text
        .split(',')
        .map(|part| part.trim().parse::<f32>())
        .collect::<Result<_, _>>()
        .map_err(|_| format!("\"{text}\" is not four numbers separated by commas"))?;
    let [left, top, width, height] = numbers[..] else {
        return Err(format!(
            "\"{text}\" has {} numbers: give left,top,width,height",
            numbers.len()
        ));
    };
    let has_a_size = width > 0.0 && height > 0.0;
    if !has_a_size {
        return Err("the width and the height must be greater than zero".to_string());
    }
    Ok(RectangleArgument {
        left,
        top,
        width,
        height,
    })
}

/// How `blur` hides the region.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum HidingStyle {
    /// A strong gaussian blur.
    Blur,
    /// Big square pixels.
    Pixelate,
    /// A black mask.
    Fill,
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
    fn blur_options_are_parsed() {
        let parsed = CommandLine::try_parse_from([
            "kadr-cli",
            "blur",
            "interview.mp4",
            "--at",
            "2.5",
            "--rect",
            "420,180,160,200",
            "-o",
            "blurred.mp4",
            "--style",
            "pixelate",
            "--debug",
        ])
        .unwrap();
        let Command::Blur {
            at,
            rect,
            style,
            debug,
            ..
        } = parsed.command
        else {
            panic!("expected the blur command");
        };
        assert_eq!(at, 2.5);
        assert_eq!(
            rect,
            RectangleArgument {
                left: 420.0,
                top: 180.0,
                width: 160.0,
                height: 200.0
            }
        );
        assert_eq!(style, HidingStyle::Pixelate);
        assert!(debug);
    }

    #[test]
    fn a_region_cut_by_the_left_edge_can_be_given() {
        let parsed = CommandLine::try_parse_from([
            "kadr-cli",
            "blur",
            "in.mp4",
            "--at",
            "1",
            "--rect",
            "-20,100,160,200",
            "-o",
            "out.mp4",
        ])
        .unwrap();
        let Command::Blur { rect, .. } = parsed.command else {
            panic!("expected the blur command");
        };
        assert_eq!(rect.left, -20.0);
    }

    #[test]
    fn bad_rectangles_are_explained() {
        assert!(parse_rectangle("1,2,3").unwrap_err().contains("3 numbers"));
        assert!(
            parse_rectangle("1,2,a,4")
                .unwrap_err()
                .contains("not four numbers")
        );
        assert!(
            parse_rectangle("1,2,0,4")
                .unwrap_err()
                .contains("greater than zero")
        );
        assert_eq!(parse_rectangle(" 1, 2, 3, 4 ").unwrap().height, 4.0);
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
