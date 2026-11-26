//! Commands that produce pictures and sound: preview frames, thumbnails,
//! waveforms, playback sound. They all answer raw bytes (see
//! [`crate::binary_messages`]) and do their work off the interface's thread.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::Receiver;

use kadr_core::ids::MediaAssetId;
use kadr_core::media_asset::{MediaAssetDescription, PictureSize};
use kadr_core::time::Timestamp;
use kadr_engine::media_overview::compute_waveform_peaks;
use tauri::State;
use tauri::ipc::Response;

use crate::app_state::{AppState, lock_shared};
use crate::binary_messages::{encode_sound, encode_waveform};
use crate::command_error::{CommandError, CommandResult, ErrorCode};
use crate::media_workers::PictureReply;

/// Preview frames are never larger than 4K, whatever the interface asks.
const LARGEST_PREVIEW_SIDE_IN_PIXELS: u32 = 3840;
/// Thumbnails are never larger than this.
const LARGEST_THUMBNAIL_SIDE_IN_PIXELS: u32 = 640;
/// Waveforms describe each second with this many peaks: one per 10 ms.
const WAVEFORM_PEAKS_PER_SECOND: u32 = 100;
/// Playback asks for sound a few seconds at a time; this bounds one answer.
const LONGEST_SOUND_REQUEST_IN_SAMPLES: u32 = 48_000 * 30;

/// The preview frame of the open project at `time_ticks`, fitting `max_width × max_height`.
#[tauri::command]
pub async fn render_preview_frame(
    state: State<'_, AppState>,
    time_ticks: u64,
    max_width: u32,
    max_height: u32,
) -> CommandResult<Response> {
    let project = state.with_open_project(|open_project| Ok(open_project.snapshot()))?;
    let answer = state.preview.request_frame(
        project,
        Timestamp::from_ticks(time_ticks),
        bounded_size(max_width, max_height, LARGEST_PREVIEW_SIDE_IN_PIXELS),
    );
    wait_for_picture(answer).await
}

/// A thumbnail of an imported file at `source_time_ticks` from its start.
#[tauri::command]
pub async fn get_thumbnail(
    state: State<'_, AppState>,
    media_id: MediaAssetId,
    source_time_ticks: u64,
    max_width: u32,
    max_height: u32,
) -> CommandResult<Response> {
    let description = media_description(&state, media_id)?;
    let answer = state.thumbnails.request_thumbnail(
        description.file_path,
        Timestamp::from_ticks(source_time_ticks),
        bounded_size(max_width, max_height, LARGEST_THUMBNAIL_SIDE_IN_PIXELS),
    );
    wait_for_picture(answer).await
}

/// The waveform of an imported file with sound (computed once, then kept).
#[tauri::command]
pub async fn get_waveform(
    state: State<'_, AppState>,
    media_id: MediaAssetId,
) -> CommandResult<Response> {
    let description = media_description(&state, media_id)?;
    let waveforms = Arc::clone(&state.waveforms);
    if let Some(encoded) = lock_shared(&waveforms).get(&description.file_path) {
        return Ok(Response::new(encoded.as_ref().clone()));
    }
    let backend = Arc::clone(&state.backend);
    let file_path: PathBuf = description.file_path.clone();
    let encoded = tauri::async_runtime::spawn_blocking(move || {
        compute_waveform_peaks(backend.as_ref(), &description, WAVEFORM_PEAKS_PER_SECOND)
            .map(|waveform| Arc::new(encode_waveform(&waveform)))
            .map_err(|error| {
                CommandError::new(ErrorCode::PreviewFailed, error)
                    .about_file(&description.file_path)
            })
    })
    .await
    .map_err(CommandError::internal)??;
    lock_shared(&waveforms).insert(file_path, Arc::clone(&encoded));
    Ok(Response::new(encoded.as_ref().clone()))
}

/// `sample_count` samples of the open project's mixed sound from `first_sample`.
#[tauri::command]
pub async fn get_timeline_sound(
    state: State<'_, AppState>,
    first_sample: u64,
    sample_count: u32,
) -> CommandResult<Response> {
    let project = state.with_open_project(|open_project| Ok(open_project.snapshot()))?;
    let backend = Arc::clone(&state.backend);
    let mixer = Arc::clone(&state.timeline_sound);
    let first_sample = usize::try_from(first_sample).map_err(CommandError::internal)?;
    let sample_count = sample_count.min(LONGEST_SOUND_REQUEST_IN_SAMPLES) as usize;
    let sound = tauri::async_runtime::spawn_blocking(move || {
        lock_shared(&mixer)
            .mix_range(&project, backend.as_ref(), first_sample, sample_count)
            .map_err(|error| CommandError::new(ErrorCode::PreviewFailed, error))
    })
    .await
    .map_err(CommandError::internal)??;
    Ok(Response::new(encode_sound(&sound)))
}

/// The description of an imported file of the open project.
fn media_description(
    state: &AppState,
    media_id: MediaAssetId,
) -> CommandResult<MediaAssetDescription> {
    state.with_open_project(|open_project| {
        open_project
            .project()
            .media_asset(media_id)
            .map(|media_asset| media_asset.description.clone())
            .ok_or_else(|| {
                CommandError::new(
                    ErrorCode::PreviewFailed,
                    format!("media {media_id} does not exist"),
                )
            })
    })
}

/// A size the interface asked for, kept between 16 pixels and `largest_side`.
fn bounded_size(width: u32, height: u32, largest_side: u32) -> PictureSize {
    const SMALLEST_SIDE_IN_PIXELS: u32 = 16;
    PictureSize {
        width_in_pixels: width.clamp(SMALLEST_SIDE_IN_PIXELS, largest_side),
        height_in_pixels: height.clamp(SMALLEST_SIDE_IN_PIXELS, largest_side),
    }
}

/// Waits (off the interface's thread) for a worker's answer.
async fn wait_for_picture(answer: Receiver<PictureReply>) -> CommandResult<Response> {
    let reply = tauri::async_runtime::spawn_blocking(move || answer.recv())
        .await
        .map_err(CommandError::internal)?
        .map_err(|_| CommandError::internal("the picture thread did not answer"))?;
    reply.map(Response::new)
}
