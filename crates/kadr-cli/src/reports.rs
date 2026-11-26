//! Human-readable text printed by the commands.

use std::io::Write;
use std::path::Path;
use std::time::Duration;

use kadr_core::media_asset::{MediaAssetDescription, MediaKind};
use kadr_core::project::Project;
use kadr_engine::{ExportProgress, ExportReport};

/// Describes a probed media file, one property per line.
pub fn describe_media(description: &MediaAssetDescription) -> String {
    let kind = match description.kind {
        MediaKind::Video => "video",
        MediaKind::Audio => "audio",
        MediaKind::Image => "image",
    };
    let mut lines = vec![
        description.display_name.clone(),
        format!("  Kind:      {kind}"),
    ];
    if let Some(duration) = description.duration {
        lines.push(format!("  Duration:  {duration}"));
    }
    if let Some(picture_size) = description.picture_size {
        let frame_rate = description
            .frame_rate
            .map(|frame_rate| format!(" at {frame_rate}"))
            .unwrap_or_default();
        lines.push(format!(
            "  Picture:   {}×{}{frame_rate}",
            picture_size.width_in_pixels, picture_size.height_in_pixels
        ));
    }
    if let Some(audio_format) = description.audio_format {
        lines.push(format!(
            "  Sound:     {} Hz, {} channel(s)",
            audio_format.sample_rate_in_hertz, audio_format.channel_count
        ));
    }
    lines.join("\n")
}

/// One line summarising a project created by `assemble`.
pub fn describe_assembled_project(project_path: &Path, project: &Project) -> String {
    let canvas = project.settings().canvas_size;
    format!(
        "Created {}: {} clip(s), {}, {}×{}",
        project_path.display(),
        project.timeline().clip_ids().len(),
        project.timeline().duration(),
        canvas.width_in_pixels,
        canvas.height_in_pixels
    )
}

/// The summary printed after an export.
pub fn describe_export(report: &ExportReport, elapsed: Duration) -> String {
    let speed_compared_to_real_time =
        report.duration.as_seconds() / elapsed.as_secs_f64().max(0.001);
    let mut lines = vec![format!(
        "Exported {}: {} frames, {}, encoder {}, in {:.1} s ({:.1}× real time)",
        report.output_path.display(),
        report.frame_count,
        report.duration,
        report.video_encoder_name,
        elapsed.as_secs_f64(),
        speed_compared_to_real_time
    )];
    lines.extend(
        report
            .warnings
            .iter()
            .map(|warning| format!("  note: {warning}")),
    );
    lines.join("\n")
}

/// A progress line on the error output, rewritten in place when the
/// percentage changes.
#[derive(Debug, Default)]
pub struct ProgressLine {
    last_shown_percentage: Option<u64>,
}

impl ProgressLine {
    /// Shows the new progress if the percentage changed.
    pub fn update(&mut self, progress: ExportProgress) {
        let percentage = (progress.fraction() * 100.0).floor() as u64;
        if self.last_shown_percentage == Some(percentage) {
            return;
        }
        self.last_shown_percentage = Some(percentage);
        eprint!(
            "\rExporting {percentage:>3}% ({}/{} frames)",
            progress.encoded_frame_count, progress.total_frame_count
        );
        // The line is redrawn in place; a failed flush only delays the display.
        let _ = std::io::stderr().flush();
    }

    /// Ends the progress line.
    pub fn finish(&mut self) {
        if self.last_shown_percentage.is_some() {
            eprintln!();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kadr_core::media_asset::{AudioFormat, PictureSize};
    use kadr_core::time::{FrameRate, TimeSpan};
    use std::path::PathBuf;

    #[test]
    fn media_descriptions_list_every_known_property() {
        let description = MediaAssetDescription {
            file_path: PathBuf::from("clip.mp4"),
            display_name: "clip.mp4".to_string(),
            kind: MediaKind::Video,
            duration: Some(TimeSpan::from_milliseconds(12_500)),
            picture_size: Some(PictureSize {
                width_in_pixels: 1920,
                height_in_pixels: 1080,
            }),
            frame_rate: Some(FrameRate::FPS_30),
            audio_format: Some(AudioFormat {
                sample_rate_in_hertz: 48_000,
                channel_count: 2,
            }),
        };
        assert_eq!(
            describe_media(&description),
            "clip.mp4\n  Kind:      video\n  Duration:  12.500 s\n  Picture:   1920×1080 at 30 fps\n  Sound:     48000 Hz, 2 channel(s)"
        );
    }

    #[test]
    fn export_summaries_include_warnings() {
        let report = ExportReport {
            output_path: PathBuf::from("out.mp4"),
            frame_count: 150,
            duration: TimeSpan::from_whole_seconds(5),
            video_encoder_name: "libx264".to_string(),
            warnings: vec!["text clip 4 was not drawn".to_string()],
        };
        let summary = describe_export(&report, Duration::from_secs(2));
        assert!(summary.starts_with(
            "Exported out.mp4: 150 frames, 5.000 s, encoder libx264, in 2.0 s (2.5× real time)"
        ));
        assert!(summary.ends_with("  note: text clip 4 was not drawn"));
    }
}
