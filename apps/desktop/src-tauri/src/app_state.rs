//! Everything the app keeps while it runs, shared by the Tauri commands.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use kadr_engine::timeline_sound::TimelineSoundMixer;
use kadr_media::{FfmpegBackend, MediaBackend};

use crate::claude_link::ClaudeToolbox;
use crate::command_error::{CommandError, CommandResult};
use crate::export_job::ExportJobs;
use crate::media_workers::{PreviewWorker, ThumbnailWorker};
use crate::open_project::OpenProject;
use crate::proxy_jobs::{ProxyJobs, ProxyStatusSender};
use crate::silence_search::SilenceSearches;
use crate::tracking_job::TrackingJobs;

/// The app's state, managed by Tauri.
pub struct AppState {
    /// Reads and writes media files.
    pub backend: Arc<dyn MediaBackend>,
    /// The project being edited, if any.
    pub open_project: Mutex<Option<OpenProject>>,
    /// Renders preview frames on its own thread.
    pub preview: PreviewWorker,
    /// Makes thumbnails on its own thread.
    pub thumbnails: ThumbnailWorker,
    /// Mixes playback sound, keeping decoded clips.
    pub timeline_sound: Arc<Mutex<TimelineSoundMixer>>,
    /// Encoded waveforms, by file.
    pub waveforms: Arc<Mutex<HashMap<PathBuf, Arc<Vec<u8>>>>>,
    /// Where the recent-projects list lives.
    pub configuration_folder: PathBuf,
    /// Where new projects are created (`Documents/Kadr`).
    pub projects_folder: PathBuf,
    /// The export running in the background, and the files exports wrote.
    pub exports: ExportJobs,
    /// The region being followed for a blur, if any.
    pub tracking: TrackingJobs,
    /// The silence dialog's searches, so a newer one stops an older one.
    pub silence_searches: SilenceSearches,
    /// Runs Claude's tool calls on the open project (decision record 0030).
    pub claude: ClaudeToolbox,
    /// Proxies of heavy videos, made in the background (decision record 0055).
    pub proxies: ProxyJobs,
}

impl AppState {
    /// Starts the media backend and the worker threads. Proxies go in
    /// `proxy_cache_folder`; their states are sent through `send_proxy_status`.
    ///
    /// # Errors
    ///
    /// When FFmpeg cannot be initialised or a thread cannot start.
    pub fn new(
        configuration_folder: PathBuf,
        projects_folder: PathBuf,
        proxy_cache_folder: PathBuf,
        send_proxy_status: ProxyStatusSender,
    ) -> CommandResult<Self> {
        let backend: Arc<dyn MediaBackend> =
            Arc::new(FfmpegBackend::new().map_err(CommandError::internal)?);
        let proxies = ProxyJobs::start(
            Arc::clone(&backend),
            &configuration_folder,
            proxy_cache_folder,
            send_proxy_status,
        )
        .map_err(CommandError::internal)?;
        Ok(Self {
            preview: PreviewWorker::start(Arc::clone(&backend), proxies.ready_proxies())
                .map_err(CommandError::internal)?,
            thumbnails: ThumbnailWorker::start(Arc::clone(&backend), proxies.ready_proxies())
                .map_err(CommandError::internal)?,
            claude: ClaudeToolbox::new(Arc::clone(&backend)),
            backend,
            open_project: Mutex::new(None),
            timeline_sound: Arc::new(Mutex::new(TimelineSoundMixer::new())),
            waveforms: Arc::new(Mutex::new(HashMap::new())),
            configuration_folder,
            projects_folder,
            exports: ExportJobs::default(),
            tracking: TrackingJobs::default(),
            silence_searches: SilenceSearches::default(),
            proxies,
        })
    }

    /// Looks for videos of the open project that need a proxy (after opening a
    /// project or importing media, by the user or by Claude).
    pub fn notice_media_of_open_project(&self) {
        let project = self.lock_open_project().as_ref().map(OpenProject::snapshot);
        if let Some(project) = project {
            self.proxies.notice_media_of(&project);
        }
    }

    /// The open project slot. A panic while it was held cannot leave a half-applied
    /// edit (edits are all or nothing), so a poisoned lock is simply taken over.
    pub fn lock_open_project(&self) -> MutexGuard<'_, Option<OpenProject>> {
        self.open_project
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Runs `action` on the open project.
    ///
    /// # Errors
    ///
    /// When no project is open, or `action` fails.
    pub fn with_open_project<Output>(
        &self,
        action: impl FnOnce(&mut OpenProject) -> CommandResult<Output>,
    ) -> CommandResult<Output> {
        let mut slot = self.lock_open_project();
        let open_project = slot.as_mut().ok_or_else(CommandError::no_project_open)?;
        action(open_project)
    }
}

/// Locks a mutex shared with blocking tasks, taking it over if a task panicked.
pub fn lock_shared<Value>(mutex: &Mutex<Value>) -> MutexGuard<'_, Value> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
