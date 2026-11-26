//! Human-readable text printed by the commands.

use std::io::Write;
use std::path::Path;
use std::time::Duration;

use kadr_core::effect::TrackingConfidence;
use kadr_core::media_asset::{MediaAssetDescription, MediaKind};
use kadr_core::project::Project;
use kadr_core::time::{FrameRate, TICKS_PER_SECOND, Timestamp};
use kadr_engine::{ExportProgress, ExportReport, RegionTrackingReport, TrackingProgress};

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
        "Exported {}: {} frames, {}, rendered by {}, encoder {}, in {:.1} s ({:.1}× real time)",
        report.output_path.display(),
        report.frame_count,
        report.duration,
        report.renderer_description,
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

/// The summary printed after tracking a region.
pub fn describe_tracking(
    report: &RegionTrackingReport,
    elapsed: Duration,
    frame_rate: FrameRate,
) -> String {
    let frame_count = report.frames.len();
    let seconds = elapsed.as_secs_f64().max(0.001);
    let video_seconds = frame_count as f64 / frame_rate.as_frames_per_second();
    let (confident, uncertain, lost) = report.confidence_counts();
    let mut lines = vec![format!(
        "Tracked {frame_count} frames in {seconds:.1} s ({:.0} frames/s, {:.1}× real time): \
         {confident} confident, {uncertain} uncertain, {lost} lost; {} samples kept",
        frame_count as f64 / seconds,
        video_seconds / seconds,
        report.region_samples.len()
    )];
    lines.extend(report.unsure_ranges.iter().map(|range| {
        let confidence = match range.confidence {
            TrackingConfidence::Lost => "lost",
            _ => "uncertain",
        };
        let first_frame = frame_rate.frame_index_at(range.first_source_time);
        let last_frame = frame_rate.frame_index_at(range.last_source_time);
        // From the start of the first unsure frame to the end of the last one.
        let end = frame_rate.timestamp_of_frame(last_frame + 1);
        let frames = if first_frame == last_frame {
            format!("frame {first_frame}")
        } else {
            format!("frames {first_frame} to {last_frame}")
        };
        format!(
            "  check: {frames}, {} to {} ({confidence})",
            seconds_rounded_up(range.first_source_time),
            seconds_rounded_up(end)
        )
    }));
    lines.join("\n")
}

/// `1.034 s`: rounded **up** to the millisecond, so that the printed time is
/// inside the frame that starts at `time` (frames last more than a millisecond).
fn seconds_rounded_up(time: Timestamp) -> String {
    const TICKS_PER_MILLISECOND: u64 = TICKS_PER_SECOND / 1_000;
    let milliseconds = time.ticks().div_ceil(TICKS_PER_MILLISECOND);
    format!("{}.{:03} s", milliseconds / 1_000, milliseconds % 1_000)
}

/// A progress line on the error output, rewritten in place when the
/// percentage changes.
#[derive(Debug, Default)]
pub struct ProgressLine {
    last_shown_percentage: Option<u64>,
}

impl ProgressLine {
    /// Shows the export's progress if the percentage changed.
    pub fn update(&mut self, progress: ExportProgress) {
        self.show(
            "Exporting",
            progress.fraction(),
            progress.encoded_frame_count,
            progress.total_frame_count,
        );
    }

    /// Shows the progress of a tracking task, named `action`.
    pub fn update_tracking(&mut self, action: &str, progress: TrackingProgress) {
        self.show(
            action,
            progress.fraction(),
            progress.analysed_frame_count,
            progress.total_frame_count,
        );
    }

    fn show(&mut self, action: &str, fraction: f64, done_frame_count: u64, total_frame_count: u64) {
        let percentage = (fraction * 100.0).floor() as u64;
        if self.last_shown_percentage == Some(percentage) {
            return;
        }
        self.last_shown_percentage = Some(percentage);
        eprint!("\r{action} {percentage:>3}% ({done_frame_count}/{total_frame_count} frames)");
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

    #[test]
    fn printed_times_stay_inside_their_frame() {
        // Frame 31 at 30 fps starts at 1.0333… s: "1.034 s", not "1.033 s".
        let start = FrameRate::FPS_30.timestamp_of_frame(31);
        assert_eq!(seconds_rounded_up(start), "1.034 s");
        let printed = Timestamp::from_seconds(1.034).unwrap();
        assert_eq!(FrameRate::FPS_30.frame_index_at(printed), 31);
        assert_eq!(seconds_rounded_up(Timestamp::ZERO), "0.000 s");
    }
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
            renderer_description: "CPU".to_string(),
            warnings: vec!["text clip 4 was not drawn".to_string()],
        };
        let summary = describe_export(&report, Duration::from_secs(2));
        assert!(summary.starts_with(
            "Exported out.mp4: 150 frames, 5.000 s, rendered by CPU, encoder libx264, in 2.0 s (2.5× real time)"
        ));
        assert!(summary.ends_with("  note: text clip 4 was not drawn"));
    }
}
