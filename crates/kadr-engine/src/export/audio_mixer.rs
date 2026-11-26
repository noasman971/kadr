//! Mixing the sound of the whole timeline.
//!
//! Every audible clip (on a track that is not muted) has its range of sound
//! decoded at its playback speed, at exactly the clip's length, then added into
//! one stereo buffer at the clip's position with the clip's volume (animated
//! if it has keyframes).

use kadr_core::clip::{Clip, ClipContent};
use kadr_core::project::Project;
use kadr_core::time::Timestamp;
use kadr_media::audio_buffer::{sample_index_at, span_of_sample_count};
use kadr_media::{AudioBuffer, AudioSegmentRequest, MediaBackend};

use super::{ExportCancellation, ExportError};

/// The mixed sound of the whole timeline, at the project's sample rate.
pub(super) fn mix_timeline_sound(
    project: &Project,
    backend: &dyn MediaBackend,
    cancellation: &ExportCancellation,
) -> Result<AudioBuffer, ExportError> {
    let sample_rate = project.settings().audio_sample_rate_in_hertz;
    let timeline_end = Timestamp::ZERO + project.timeline().duration();
    let mut mix = AudioBuffer::silence(sample_rate, sample_index_at(timeline_end, sample_rate));

    let audible_tracks = project
        .timeline()
        .tracks()
        .iter()
        .filter(|track| !track.is_muted);
    for track in audible_tracks {
        for clip in track.clips() {
            if cancellation.is_cancelled() {
                return Err(ExportError::Cancelled);
            }
            if let Some(request) = sound_request_for(project, clip, sample_rate) {
                let first_sample = sample_index_at(clip.timeline_start, sample_rate);
                let clip_sound = backend.decode_audio_segment(&request)?;
                mix_clip_sound(&mut mix, &clip_sound, first_sample, clip);
            }
        }
    }
    mix.clamp_to_full_scale();
    Ok(mix)
}

/// What to decode for this clip, or `None` if the clip has no sound.
fn sound_request_for(
    project: &Project,
    clip: &Clip,
    sample_rate: u32,
) -> Option<AudioSegmentRequest> {
    let ClipContent::Media(segment) = &clip.content else {
        return None;
    };
    let media_asset = project.media_asset(segment.media_asset_id)?;
    if !media_asset.description.has_audio() {
        return None;
    }
    // Start and end positions are rounded the same way for every clip, so two
    // clips placed end to end leave no gap and no overlap between their sounds.
    let first_sample = sample_index_at(clip.timeline_start, sample_rate);
    let end_sample = sample_index_at(clip.timeline_end(), sample_rate);
    Some(AudioSegmentRequest {
        file_path: media_asset.description.file_path.clone(),
        source_start: segment.source_start,
        source_end: segment.source_end,
        speed: segment.speed,
        output_sample_rate_in_hertz: sample_rate,
        output_samples_per_channel: end_sample.saturating_sub(first_sample),
    })
}

/// Adds the clip's sound into the mix with its (possibly animated) volume.
fn mix_clip_sound(
    mix: &mut AudioBuffer,
    clip_sound: &AudioBuffer,
    first_sample: usize,
    clip: &Clip,
) {
    let volume = &clip.volume_multiplier;
    let sample_rate = mix.sample_rate_in_hertz();
    if volume.is_animated() {
        mix.mix_in(clip_sound, first_sample, |sample_index| {
            volume.value_at(span_of_sample_count(sample_index, sample_rate))
        });
    } else {
        let constant_volume = volume.constant_value;
        mix.mix_in(clip_sound, first_sample, |_| constant_volume);
    }
}
