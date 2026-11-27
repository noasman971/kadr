//! Proxies of heavy videos, made in the background (decision record 0055,
//! docs/desktop-app.md "Proxies").
//!
//! When a project is opened or media are imported (by the user or by Claude),
//! the app *notices* its videos. One worker thread then, for each new video:
//!
//! 1. uses its proxy if the cache already has one for the file as it is now;
//! 2. otherwise, if automatic proxies are on and the rule says the video is heavy
//!    ([`proxy_need_of`]), queues a proxy and makes it, one at a time, with its
//!    progress sent to the front end (`proxy-changed` events);
//! 3. once made, lists it in [`ReadyProxies`], which the preview and thumbnail
//!    threads read, and trims the cache to [`PROXY_CACHE_LIMIT_IN_BYTES`].
//!
//! The user can stop a proxy, make it again, turn automatic proxies off (then
//! none is made or used) and clear the cache. Nothing of this goes into the
//! project file. The setting lives in `proxy-settings.json` in the app's
//! configuration folder; proxies in the app's cache folder.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use kadr_core::media_asset::{MediaAssetDescription, MediaKind};
use kadr_core::project::Project;
use kadr_engine::proxies::{
    ProxyCache, ProxyMakingOutcome, ProxyReason, ReadyProxies, ReadyProxy, proxy_need_of,
};
use kadr_media::MediaBackend;
use serde::{Deserialize, Serialize};

use crate::command_error::{CommandError, CommandResult};

/// The cache is trimmed to this size after each new proxy, least recently used
/// first (proxies of the session are kept). 1080p proxies take about 80 MB
/// per minute: this is about two hours of video.
pub const PROXY_CACHE_LIMIT_IN_BYTES: u64 = 10_000_000_000;
/// Progress is sent at most this often.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);
/// The file holding the setting, in the app's configuration folder.
const SETTINGS_FILE_NAME: &str = "proxy-settings.json";

/// Where a video stands, as the media library shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ProxyState {
    /// Waiting for the proxies before it.
    Queued {
        /// Why it gets one; `None` when the user asked for it.
        reason: Option<ProxyReason>,
    },
    /// Being made.
    Making {
        /// Why it gets one; `None` when the user asked for it.
        reason: Option<ProxyReason>,
        /// From 0 to 1.
        fraction: f64,
    },
    /// The preview shows the proxy.
    Ready {
        /// Width of the proxy's pictures.
        width: u32,
        /// Height of the proxy's pictures.
        height: u32,
    },
    /// The user stopped it; it can be made again.
    Stopped,
    /// It could not be made; it can be tried again.
    Failed {
        /// Why, in the engine's words.
        details: String,
    },
}

/// One video's proxy state, as sent to the front end.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyStatus {
    /// The video, as the project names it (`MediaView::file_path`).
    pub file_path: String,
    /// Its proxy's state; `None` when it has none and gets none.
    pub proxy: Option<ProxyState>,
}

/// Everything the settings and the library need at once.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyOverview {
    /// Whether heavy videos get proxies automatically (and proxies are used).
    pub is_automatic: bool,
    /// The space the proxies take on disk.
    pub cache_size_in_bytes: u64,
    /// Where they are.
    pub cache_folder: String,
    /// The state of every video known in this session.
    pub statuses: Vec<ProxyStatus>,
}

/// The stored setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProxySettings {
    is_automatic: bool,
}

impl Default for ProxySettings {
    fn default() -> Self {
        Self { is_automatic: true }
    }
}

/// What the worker shares with the commands.
#[derive(Default)]
struct ProxyBoard {
    is_automatic: bool,
    /// Videos already looked at in this session.
    noticed: HashSet<PathBuf>,
    /// Videos to look at (cache, rule).
    to_check: VecDeque<MediaAssetDescription>,
    /// Proxies waiting to be made, with why (`None`: the user asked).
    queue: VecDeque<(MediaAssetDescription, Option<ProxyReason>)>,
    /// The proxy being made, and what stops it.
    current: Option<(PathBuf, Arc<AtomicBool>)>,
    /// What the library shows, by video.
    states: HashMap<PathBuf, ProxyState>,
}

impl ProxyBoard {
    fn has_work(&self) -> bool {
        !self.to_check.is_empty() || !self.queue.is_empty()
    }
}

/// Sends a status to the front end.
pub type ProxyStatusSender = Box<dyn Fn(ProxyStatus) + Send + Sync>;

/// What the worker thread and the commands share.
struct ProxyShared {
    board: Mutex<ProxyBoard>,
    work_arrived: Condvar,
    cache: ProxyCache,
    ready: Arc<ReadyProxies>,
    backend: Arc<dyn MediaBackend>,
    send_status: ProxyStatusSender,
}

impl ProxyShared {
    fn lock_board(&self) -> MutexGuard<'_, ProxyBoard> {
        self.board.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Records `state` for `file_path` and tells the front end.
    fn set_state(&self, file_path: &Path, state: Option<ProxyState>) {
        {
            let mut board = self.lock_board();
            match &state {
                Some(state) => board.states.insert(file_path.to_path_buf(), state.clone()),
                None => board.states.remove(file_path),
            };
        }
        (self.send_status)(ProxyStatus {
            file_path: file_path.display().to_string(),
            proxy: state,
        });
    }
}

/// The proxies of the session: the worker thread and its controls.
pub struct ProxyJobs {
    shared: Arc<ProxyShared>,
    settings_path: PathBuf,
}

impl ProxyJobs {
    /// Starts the worker thread, with the setting stored in
    /// `configuration_folder` and the proxies in `cache_folder`.
    ///
    /// # Errors
    ///
    /// When the system cannot start a thread.
    pub fn start(
        backend: Arc<dyn MediaBackend>,
        configuration_folder: &Path,
        cache_folder: PathBuf,
        send_status: ProxyStatusSender,
    ) -> std::io::Result<Self> {
        let settings_path = configuration_folder.join(SETTINGS_FILE_NAME);
        let settings = load_settings(&settings_path);
        let ready = Arc::new(ReadyProxies::new());
        ready.set_enabled(settings.is_automatic);
        let shared = Arc::new(ProxyShared {
            board: Mutex::new(ProxyBoard {
                is_automatic: settings.is_automatic,
                ..ProxyBoard::default()
            }),
            work_arrived: Condvar::new(),
            cache: ProxyCache::new(cache_folder),
            ready,
            backend,
            send_status,
        });
        let worker_shared = Arc::clone(&shared);
        thread::Builder::new()
            .name("kadr-proxies".to_string())
            .spawn(move || run_proxy_worker(&worker_shared))?;
        Ok(Self {
            shared,
            settings_path,
        })
    }

    /// The proxies the preview and thumbnails may read.
    pub fn ready_proxies(&self) -> Arc<ReadyProxies> {
        Arc::clone(&self.shared.ready)
    }

    /// Looks at the videos of `project` not seen yet in this session.
    pub fn notice_media_of(&self, project: &Project) {
        let mut board = self.shared.lock_board();
        let mut has_new_media = false;
        for media_asset in project.media_assets() {
            let description = &media_asset.description;
            if description.kind != MediaKind::Video
                || board.noticed.contains(&description.file_path)
            {
                continue;
            }
            board.noticed.insert(description.file_path.clone());
            board.to_check.push_back(description.clone());
            has_new_media = true;
        }
        if has_new_media {
            self.shared.work_arrived.notify_one();
        }
    }

    /// Stops the proxy of `file_path` (queued or being made).
    pub fn stop(&self, file_path: &Path) {
        let was_queued = {
            let mut board = self.shared.lock_board();
            if let Some((current_path, stop)) = &board.current
                && current_path == file_path
            {
                stop.store(true, Ordering::Relaxed);
            }
            let queue_length = board.queue.len();
            board
                .queue
                .retain(|(description, _)| description.file_path != file_path);
            board.queue.len() != queue_length
        };
        if was_queued {
            self.shared.set_state(file_path, Some(ProxyState::Stopped));
        }
    }

    /// Makes the proxy of the video `description` describes, even if it was
    /// stopped or failed, or the rule would not ask for one.
    pub fn make_again(&self, description: MediaAssetDescription) -> CommandResult<()> {
        if description.kind != MediaKind::Video {
            return Err(CommandError::internal(format!(
                "\"{}\" is not a video",
                description.file_path.display()
            )));
        }
        let file_path = description.file_path.clone();
        {
            let mut board = self.shared.lock_board();
            let is_busy_with_it = board
                .current
                .as_ref()
                .is_some_and(|(current_path, _)| *current_path == file_path)
                || board
                    .queue
                    .iter()
                    .any(|(queued, _)| queued.file_path == file_path);
            if is_busy_with_it {
                return Ok(());
            }
            board.noticed.insert(file_path.clone());
            board.queue.push_back((description, None));
        }
        self.shared
            .set_state(&file_path, Some(ProxyState::Queued { reason: None }));
        self.shared.work_arrived.notify_one();
        Ok(())
    }

    /// Turns automatic proxies on or off, and remembers it. Off, nothing is made
    /// and the preview reads the originals; on, the open project's videos are
    /// looked at again (`project`).
    pub fn set_automatic(
        &self,
        is_automatic: bool,
        project: Option<&Project>,
    ) -> CommandResult<()> {
        save_settings(&self.settings_path, ProxySettings { is_automatic })?;
        self.shared.ready.set_enabled(is_automatic);
        let paths_to_forget: Vec<PathBuf> = {
            let mut board = self.shared.lock_board();
            if board.is_automatic == is_automatic {
                return Ok(());
            }
            board.is_automatic = is_automatic;
            board.noticed.clear();
            board.to_check.clear();
            if let Some((_, stop)) = &board.current {
                stop.store(true, Ordering::Relaxed);
            }
            let mut paths: Vec<PathBuf> = board
                .queue
                .drain(..)
                .map(|(queued, _)| queued.file_path)
                .collect();
            paths.extend(board.states.keys().cloned());
            paths
        };
        if !is_automatic {
            for file_path in paths_to_forget {
                self.shared.set_state(&file_path, None);
            }
        }
        if let Some(project) = project
            && is_automatic
        {
            self.notice_media_of(project);
        }
        Ok(())
    }

    /// The setting, the cache's size and every state.
    pub fn overview(&self) -> ProxyOverview {
        let (is_automatic, statuses) = {
            let board = self.shared.lock_board();
            let statuses = board
                .states
                .iter()
                .map(|(file_path, state)| ProxyStatus {
                    file_path: file_path.display().to_string(),
                    proxy: Some(state.clone()),
                })
                .collect();
            (board.is_automatic, statuses)
        };
        // Listing the folder takes a moment: not while the worker waits for the board.
        ProxyOverview {
            is_automatic,
            cache_size_in_bytes: self.shared.cache.size_in_bytes(),
            cache_folder: self.shared.cache.folder().display().to_string(),
            statuses,
        }
    }

    /// Stops every proxy and forgets them all, so their files can go: call
    /// [`Self::delete_cache_files`] once the preview and thumbnail threads have
    /// closed them. Videos are looked at again when next noticed.
    pub fn forget_every_proxy(&self) {
        let paths_shown: Vec<PathBuf> = {
            let mut board = self.shared.lock_board();
            if let Some((_, stop)) = &board.current {
                stop.store(true, Ordering::Relaxed);
            }
            board.queue.clear();
            board.to_check.clear();
            board.noticed.clear();
            board.states.keys().cloned().collect()
        };
        self.shared.ready.clear();
        for file_path in paths_shown {
            self.shared.set_state(&file_path, None);
        }
    }

    /// Deletes every proxy file (those still open elsewhere stay). Returns the
    /// space freed. Slow on big caches: call it off the interface's thread.
    pub fn delete_cache_files(&self) -> u64 {
        self.shared.cache.remove_proxies_except(&HashSet::new())
    }
}

fn load_settings(settings_path: &Path) -> ProxySettings {
    fs::read(settings_path)
        .ok()
        .and_then(|text| serde_json::from_slice(&text).ok())
        .unwrap_or_default()
}

fn save_settings(settings_path: &Path, settings: ProxySettings) -> CommandResult<()> {
    let text = serde_json::to_vec_pretty(&settings).map_err(CommandError::internal)?;
    if let Some(folder) = settings_path.parent() {
        fs::create_dir_all(folder).map_err(CommandError::internal)?;
    }
    let temporary_path = settings_path.with_extension("json.saving");
    fs::write(&temporary_path, text).map_err(CommandError::internal)?;
    fs::rename(&temporary_path, settings_path).map_err(CommandError::internal)
}

/// What the worker does next.
enum ProxyWork {
    Check(MediaAssetDescription),
    Make(MediaAssetDescription, Option<ProxyReason>, Arc<AtomicBool>),
}

fn run_proxy_worker(shared: &ProxyShared) {
    loop {
        let work = {
            let mut board = shared.lock_board();
            while !board.has_work() {
                board = shared
                    .work_arrived
                    .wait(board)
                    .unwrap_or_else(PoisonError::into_inner);
            }
            if let Some(description) = board.to_check.pop_front() {
                ProxyWork::Check(description)
            } else if let Some((description, reason)) = board.queue.pop_front() {
                let stop = Arc::new(AtomicBool::new(false));
                board.current = Some((description.file_path.clone(), Arc::clone(&stop)));
                ProxyWork::Make(description, reason, stop)
            } else {
                continue;
            }
        };
        match work {
            ProxyWork::Check(description) => check_video(shared, description),
            ProxyWork::Make(description, reason, stop) => {
                make_proxy(shared, &description, reason, &stop);
                shared.lock_board().current = None;
            }
        }
    }
}

/// Uses the cache's proxy of a video, or queues one if the video is heavy.
/// With automatic proxies off, nothing: the video is looked at again when they
/// are turned on.
fn check_video(shared: &ProxyShared, description: MediaAssetDescription) {
    if !shared.lock_board().is_automatic {
        return;
    }
    let file_path = description.file_path.clone();
    if let Some(proxy) = shared.cache.ready_proxy_of(&file_path) {
        // Proxies forgotten meanwhile (the cache is being cleared) stay forgotten.
        if !shared.lock_board().noticed.contains(&file_path) {
            return;
        }
        let state = ready_state_of(&proxy);
        shared.ready.add(proxy);
        shared.set_state(&file_path, Some(state));
        return;
    }
    // An unreadable or missing file simply gets no proxy.
    let Ok(details) = shared.backend.describe_video_stream(&file_path) else {
        return;
    };
    let Some(reason) = proxy_need_of(&details) else {
        return;
    };
    {
        let mut board = shared.lock_board();
        if !board.is_automatic {
            return;
        }
        board.queue.push_back((description, Some(reason)));
    }
    shared.set_state(
        &file_path,
        Some(ProxyState::Queued {
            reason: Some(reason),
        }),
    );
}

fn ready_state_of(proxy: &ReadyProxy) -> ProxyState {
    ProxyState::Ready {
        width: proxy.picture_size.width_in_pixels,
        height: proxy.picture_size.height_in_pixels,
    }
}

/// Makes one proxy, sending its progress, then lists it.
fn make_proxy(
    shared: &ProxyShared,
    description: &MediaAssetDescription,
    reason: Option<ProxyReason>,
    stop: &AtomicBool,
) {
    let file_path = description.file_path.as_path();
    shared.set_state(
        file_path,
        Some(ProxyState::Making {
            reason,
            fraction: 0.0,
        }),
    );
    let mut last_sent = Instant::now();
    let outcome = shared
        .cache
        .make_proxy(shared.backend.as_ref(), description, |progress| {
            if stop.load(Ordering::Relaxed) {
                return ControlFlow::Break(());
            }
            if last_sent.elapsed() >= PROGRESS_INTERVAL {
                last_sent = Instant::now();
                shared.set_state(
                    file_path,
                    Some(ProxyState::Making {
                        reason,
                        fraction: progress.fraction(),
                    }),
                );
            }
            ControlFlow::Continue(())
        });
    let is_automatic = shared.lock_board().is_automatic;
    match outcome {
        Ok(ProxyMakingOutcome::Ready(proxy)) => {
            let state = ready_state_of(&proxy);
            shared.ready.add(proxy);
            shared.set_state(file_path, Some(state));
            shared
                .cache
                .trim_to_size(PROXY_CACHE_LIMIT_IN_BYTES, &shared.ready.proxy_paths());
        }
        // Stopped by turning proxies off or clearing the cache: nothing to show.
        Ok(ProxyMakingOutcome::Cancelled) if !is_automatic => shared.set_state(file_path, None),
        Ok(ProxyMakingOutcome::Cancelled) => {
            let was_forgotten = !shared.lock_board().noticed.contains(file_path);
            let state = (!was_forgotten).then_some(ProxyState::Stopped);
            shared.set_state(file_path, state);
        }
        Err(error) => shared.set_state(
            file_path,
            Some(ProxyState::Failed {
                details: error.to_string(),
            }),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_setting_is_on_until_turned_off() {
        let folder = tempfile::tempdir().unwrap();
        let settings_path = folder.path().join(SETTINGS_FILE_NAME);
        assert!(load_settings(&settings_path).is_automatic);
        save_settings(
            &settings_path,
            ProxySettings {
                is_automatic: false,
            },
        )
        .unwrap();
        assert!(!load_settings(&settings_path).is_automatic);
    }

    #[test]
    fn states_are_sent_as_the_front_end_reads_them() {
        let status = ProxyStatus {
            file_path: "C:/videos/a.mp4".to_string(),
            proxy: Some(ProxyState::Making {
                reason: Some(ProxyReason::LargePicture),
                fraction: 0.5,
            }),
        };
        assert_eq!(
            serde_json::to_value(&status).unwrap(),
            serde_json::json!({
                "filePath": "C:/videos/a.mp4",
                "proxy": { "state": "making", "reason": "largePicture", "fraction": 0.5 }
            })
        );
    }
}
