//! The mixed sound of a part of the timeline, for playback in an interface.
//!
//! The export mixes the whole timeline at once. Playback needs a few seconds at a
//! time, starting anywhere, again and again while the user plays, stops and edits.
//! [`TimelineSoundMixer`] mixes any range **exactly like the export** (same
//! decoding request per clip, same sample positions, same volumes), and keeps the
//! decoded sound of recent clips so playing again does not decode again.
//!
//! Each clip's sound is decoded whole the first time it is heard: decoding a range
//! in the middle of a clip would restart the time-stretching filter at every chunk
//! and could click at the joins.

use std::sync::Arc;

use kadr_core::project::Project;
use kadr_media::audio_buffer::sample_index_at;
use kadr_media::{AudioBuffer, AudioSegmentRequest, MediaBackend, MediaError};

use crate::export::audio_mixer::{clip_gain, sound_request_for};

/// Samples per channel of decoded sound a mixer keeps: ten minutes at 48 kHz,
/// about 230 MB. Enough for every clip of a typical short edit.
const MAXIMUM_CACHED_SAMPLES_PER_CHANNEL: usize = 48_000 * 60 * 10;

/// The decoded sound of one clip, and the request that produced it.
struct CachedClipSound {
    request: AudioSegmentRequest,
    sound: Arc<AudioBuffer>,
}

/// Mixes ranges of a project's sound, keeping recently decoded clips.
#[derive(Default)]
pub struct TimelineSoundMixer {
    /// Most recently used last.
    clip_sounds: Vec<CachedClipSound>,
}

impl TimelineSoundMixer {
    /// A mixer with nothing decoded yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// The sound of `project`'s timeline from sample `first_sample` (at the
    /// project's sample rate, counted from the start of the timeline), lasting
    /// `sample_count` samples. Past the end of the timeline, it is silence.
    ///
    /// # Errors
    ///
    /// Fails when the sound of a clip in the range cannot be decoded.
    pub fn mix_range(
        &mut self,
        project: &Project,
        backend: &dyn MediaBackend,
        first_sample: usize,
        sample_count: usize,
    ) -> Result<AudioBuffer, MediaError> {
        let sample_rate = project.settings().audio_sample_rate_in_hertz;
        let mut mix = AudioBuffer::silence(sample_rate, sample_count);
        let end_sample = first_sample + sample_count;
        let audible_tracks = project
            .timeline()
            .tracks()
            .iter()
            .filter(|track| !track.is_muted);
        for track in audible_tracks {
            for clip in track.clips() {
                let Some(request) = sound_request_for(project, clip, sample_rate) else {
                    continue;
                };
                let clip_first_sample = sample_index_at(clip.timeline_start, sample_rate);
                let clip_end_sample = clip_first_sample + request.output_samples_per_channel;
                if clip_end_sample <= first_sample || clip_first_sample >= end_sample {
                    continue;
                }
                let clip_sound = self.clip_sound(backend, request)?;
                let first_sample_in_clip = first_sample.saturating_sub(clip_first_sample);
                let first_sample_in_mix = clip_first_sample.saturating_sub(first_sample);
                let mixed_sample_count =
                    (clip_end_sample.min(end_sample)) - (clip_first_sample.max(first_sample));
                let part = AudioBuffer::from_interleaved_stereo(
                    sample_rate,
                    clip_sound
                        .interleaved_range(first_sample_in_clip, mixed_sample_count)
                        .to_vec(),
                );
                let gain = clip_gain(clip, sample_rate);
                mix.mix_in(&part, first_sample_in_mix, |sample_index| {
                    gain(first_sample_in_clip + sample_index)
                });
            }
        }
        mix.clamp_to_full_scale();
        Ok(mix)
    }

    /// The decoded sound for `request`, decoded now if it is not kept already.
    fn clip_sound(
        &mut self,
        backend: &dyn MediaBackend,
        request: AudioSegmentRequest,
    ) -> Result<Arc<AudioBuffer>, MediaError> {
        if let Some(position) = self
            .clip_sounds
            .iter()
            .position(|cached| cached.request == request)
        {
            let cached = self.clip_sounds.remove(position);
            let sound = Arc::clone(&cached.sound);
            self.clip_sounds.push(cached);
            return Ok(sound);
        }
        let sound = Arc::new(backend.decode_audio_segment(&request)?);
        self.clip_sounds.push(CachedClipSound {
            request,
            sound: Arc::clone(&sound),
        });
        self.forget_oldest_sounds_beyond_the_limit();
        Ok(sound)
    }

    /// Drops the least recently used sounds until the cache fits its limit
    /// (the sound just decoded is always kept).
    fn forget_oldest_sounds_beyond_the_limit(&mut self) {
        let mut cached_sample_count: usize = self
            .clip_sounds
            .iter()
            .map(|cached| cached.sound.samples_per_channel())
            .sum();
        while cached_sample_count > MAXIMUM_CACHED_SAMPLES_PER_CHANNEL && self.clip_sounds.len() > 1
        {
            let forgotten = self.clip_sounds.remove(0);
            cached_sample_count -= forgotten.sound.samples_per_channel();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::ExportCancellation;
    use crate::export::audio_mixer::mix_timeline_sound;
    use crate::quick_assembly::assemble_project_from_files;
    use kadr_core::clip::{AnimatableProperty, PlaybackSpeed};
    use kadr_core::commands::{SetClipPropertyValue, SetClipSpeed};
    use kadr_core::history::EditSession;
    use kadr_core::media_asset::PictureSize;
    use kadr_core::project::ProjectSettings;
    use kadr_core::time::{FrameRate, TimeSpan};
    use kadr_media::FfmpegBackend;
    use kadr_media::test_patterns::{BeepPattern, TestVideoDescription, write_test_video};

    /// Two beeping videos end to end, the second at 1.5× and half volume.
    fn two_clip_project(backend: &FfmpegBackend, directory: &std::path::Path) -> Project {
        let duration = TimeSpan::from_whole_seconds(2);
        let video = directory.join("beeps.mp4");
        write_test_video(
            backend,
            &TestVideoDescription {
                output_path: video.clone(),
                width_in_pixels: 64,
                height_in_pixels: 48,
                frame_rate: FrameRate::FPS_30,
                duration,
                audio_sample_rate_in_hertz: 44_100,
                sound: BeepPattern::every_second(
                    880.0,
                    0.6,
                    duration,
                    TimeSpan::from_milliseconds(300),
                ),
            },
        )
        .unwrap();
        let project = assemble_project_from_files(
            backend,
            &[video.clone(), video],
            ProjectSettings {
                canvas_size: PictureSize {
                    width_in_pixels: 64,
                    height_in_pixels: 48,
                },
                ..ProjectSettings::landscape_1080p()
            },
            duration,
        )
        .unwrap();
        let mut session = EditSession::new(project).unwrap();
        let second_clip = session.project().timeline().clip_ids()[1];
        session
            .execute(SetClipSpeed::new(
                second_clip,
                PlaybackSpeed::from_fraction(3, 2).unwrap(),
            ))
            .unwrap();
        session
            .execute(SetClipPropertyValue::new(
                second_clip,
                AnimatableProperty::Volume,
                0.5,
            ))
            .unwrap();
        session.into_project()
    }

    #[test]
    fn ranges_mixed_one_after_another_equal_the_export_mix() {
        let directory = tempfile::tempdir().unwrap();
        let backend = FfmpegBackend::new().unwrap();
        let project = two_clip_project(&backend, directory.path());
        let export_mix =
            mix_timeline_sound(&project, &backend, &ExportCancellation::new()).unwrap();

        let mut mixer = TimelineSoundMixer::new();
        let mut playback_samples = Vec::new();
        // An uneven chunk length, so chunk edges fall inside clips and at odd places.
        let chunk_length = 7_919;
        let mut first_sample = 0;
        while first_sample < export_mix.samples_per_channel() {
            let chunk = mixer
                .mix_range(&project, &backend, first_sample, chunk_length)
                .unwrap();
            playback_samples.extend_from_slice(chunk.interleaved_samples());
            first_sample += chunk_length;
        }
        playback_samples.truncate(export_mix.interleaved_samples().len());

        assert_eq!(playback_samples, export_mix.interleaved_samples());
        // Every clip was decoded once and kept.
        assert_eq!(mixer.clip_sounds.len(), 2);
    }

    #[test]
    fn past_the_end_is_silence_and_muted_tracks_are_silent() {
        let directory = tempfile::tempdir().unwrap();
        let backend = FfmpegBackend::new().unwrap();
        let project = two_clip_project(&backend, directory.path());
        let mut mixer = TimelineSoundMixer::new();

        let after_the_end = mixer
            .mix_range(&project, &backend, 48_000 * 60, 1_000)
            .unwrap();
        assert!(
            after_the_end
                .interleaved_samples()
                .iter()
                .all(|&sample| sample == 0.0)
        );

        let mut session = EditSession::new(project).unwrap();
        let track_id = session.project().timeline().track_ids()[0];
        session
            .execute(kadr_core::commands::SetTrackMuted::new(track_id, true))
            .unwrap();
        let muted = mixer
            .mix_range(session.project(), &backend, 0, 48_000)
            .unwrap();
        assert!(
            muted
                .interleaved_samples()
                .iter()
                .all(|&sample| sample == 0.0)
        );
    }
}
