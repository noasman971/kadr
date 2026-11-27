//! Proxies: smaller, easy-to-decode copies of heavy videos that the preview shows
//! instead of the originals (decision record 0055).
//!
//! - [`proxy_need_of`] is the rule saying which videos get one: larger than
//!   1920 × 1080, or estimated to decode slower than 1080p H.264 at 60 fps, or
//!   above 100 Mbit/s.
//! - [`proxy_picture_size_for`] and [`PROXY_FRAMES_PER_KEYFRAME`] are the proxy
//!   format: fitting inside 1920 × 1080, H.264 without B-frames, a keyframe every
//!   few frames, every source frame kept at its time.
//! - [`ProxyCache`] is the folder of proxies, outside every project. A proxy's
//!   name comes from its source's full path, length and modification time, and a
//!   small description next to it is checked before use: a changed or moved
//!   source never gets an old proxy.
//! - [`ReadyProxies`] lists the proxies the preview may use now. The preview
//!   ([`crate::preview`]) and thumbnails read proxies; the export, Claude's
//!   pictures and the blur analysis always read the originals, and blur regions
//!   stay in the original's pixels (the preview scales them to the proxy).
//!
//! Nothing about proxies goes into `.kadr` files: projects stay portable, and a
//! project opened on another computer simply makes its own proxies.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use kadr_core::effect::{BlurStyle, PixelRegion};
use kadr_core::media_asset::{MediaAssetDescription, MediaKind, PictureSize};
use kadr_core::time::{TimeSpan, Timestamp};
use kadr_media::{
    MediaBackend, MediaError, VideoProxyOutcome, VideoProxySettings, VideoStreamDetails,
};
use kadr_render::HiddenRegion;
use serde::{Deserialize, Serialize};

use crate::picture_scaling::size_fitting_inside;

/// Videos whose longer side is larger than this get a proxy.
pub const LARGEST_LONG_SIDE_WITHOUT_PROXY: u32 = 1920;
/// Videos whose shorter side is larger than this get a proxy.
pub const LARGEST_SHORT_SIDE_WITHOUT_PROXY: u32 = 1080;
/// The most decoding work a video may need without a proxy, in "H.264 8-bit
/// 4:2:0 pixels" per second: 1080p at 60 fps.
pub const DECODING_BUDGET_IN_PIXELS_PER_SECOND: f64 = 1920.0 * 1080.0 * 60.0;
/// Videos above this bit rate get a proxy: reading and decoding so much data per
/// frame is slow, whatever the picture size.
pub const HIGHEST_BIT_RATE_WITHOUT_PROXY: u64 = 100_000_000;
/// How much more a codec costs to decode than H.264 in software (measured
/// ratios, rounded: see decision record 0055).
const SLOWER_CODECS: [(&str, f64); 3] = [("hevc", 2.0), ("vp9", 2.0), ("av1", 2.0)];
/// Pictures other than 8-bit 4:2:0 are converted on every frame: twice the work.
const CONVERTED_PICTURE_COST_FACTOR: f64 = 2.0;
/// Frame rate assumed when a file gives none.
const ASSUMED_FRAMES_PER_SECOND: f64 = 30.0;

/// The largest proxy picture, for landscape videos (portrait ones are turned).
pub const LARGEST_PROXY_SIZE: PictureSize = PictureSize {
    width_in_pixels: 1920,
    height_in_pixels: 1080,
};
/// One keyframe at least every this many frames: a jump decodes at most this many.
pub const PROXY_FRAMES_PER_KEYFRAME: u32 = 5;
/// Changes whenever proxies are made differently: older ones are then remade.
pub const PROXY_FORMAT_VERSION: u32 = 1;
/// How long the preview trusts that a source has not changed before looking again.
const SOURCE_CHECK_INTERVAL: Duration = Duration::from_secs(2);

/// Why a video gets a proxy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ProxyReason {
    /// Its picture is larger than 1920 × 1080.
    LargePicture,
    /// Its codec, frame rate or picture format make it slower to decode than
    /// 1080p H.264 at 60 fps.
    SlowToDecode,
    /// Its bit rate is above 100 Mbit/s.
    HighBitRate,
}

/// Whether a video with these details gets a proxy, and why; `None` when it
/// decodes fast enough as it is.
pub fn proxy_need_of(details: &VideoStreamDetails) -> Option<ProxyReason> {
    let PictureSize {
        width_in_pixels: width,
        height_in_pixels: height,
    } = details.picture_size;
    let (long_side, short_side) = (width.max(height), width.min(height));
    if long_side > LARGEST_LONG_SIDE_WITHOUT_PROXY || short_side > LARGEST_SHORT_SIDE_WITHOUT_PROXY
    {
        return Some(ProxyReason::LargePicture);
    }
    if estimated_decoding_cost_of(details) > DECODING_BUDGET_IN_PIXELS_PER_SECOND {
        return Some(ProxyReason::SlowToDecode);
    }
    if details
        .bit_rate_in_bits_per_second
        .is_some_and(|bit_rate| bit_rate > HIGHEST_BIT_RATE_WITHOUT_PROXY)
    {
        return Some(ProxyReason::HighBitRate);
    }
    None
}

/// Decoding work per second, in "H.264 8-bit 4:2:0 pixels": pixels per second,
/// times the codec's cost, times two for pictures converted on every frame.
fn estimated_decoding_cost_of(details: &VideoStreamDetails) -> f64 {
    let pixels_per_frame = f64::from(details.picture_size.width_in_pixels)
        * f64::from(details.picture_size.height_in_pixels);
    let frames_per_second = details
        .frame_rate
        .map_or(ASSUMED_FRAMES_PER_SECOND, |rate| {
            rate.as_frames_per_second()
        });
    let codec_cost = SLOWER_CODECS
        .iter()
        .find(|(codec_name, _)| *codec_name == details.codec_name)
        .map_or(1.0, |(_, cost)| *cost);
    let format_cost = if details.is_eight_bit_yuv420() {
        1.0
    } else {
        CONVERTED_PICTURE_COST_FACTOR
    };
    pixels_per_frame * frames_per_second * codec_cost * format_cost
}

/// The size of the proxy of a picture of `original_size`: the largest with its
/// proportions inside 1920 × 1080 (1080 × 1920 for portrait pictures), with even
/// sides. A smaller picture keeps its size (rounded down to even sides).
pub fn proxy_picture_size_for(original_size: PictureSize) -> PictureSize {
    let PictureSize {
        width_in_pixels: width,
        height_in_pixels: height,
    } = original_size;
    let largest = if height > width {
        PictureSize {
            width_in_pixels: LARGEST_PROXY_SIZE.height_in_pixels,
            height_in_pixels: LARGEST_PROXY_SIZE.width_in_pixels,
        }
    } else {
        LARGEST_PROXY_SIZE
    };
    let fitted = size_fitting_inside(width, height, largest);
    let even_side = |side: u32| (side - side % 2).max(2);
    PictureSize {
        width_in_pixels: even_side(fitted.width_in_pixels),
        height_in_pixels: even_side(fitted.height_in_pixels),
    }
}

/// What identifies one version of a source file: its full path, its length and
/// its modification time. Moving, replacing or editing the file changes it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SourceFingerprint {
    /// The file's canonical path (symbolic links and `..` resolved).
    pub canonical_path: PathBuf,
    /// Its length.
    pub length_in_bytes: u64,
    /// Its modification time, in nanoseconds since 1970.
    pub modified_nanoseconds: u128,
}

impl SourceFingerprint {
    /// The fingerprint of the file at `file_path` as it is now.
    ///
    /// # Errors
    ///
    /// When the file cannot be found or read.
    pub fn of(file_path: &Path) -> io::Result<Self> {
        let canonical_path = dunce::canonicalize(file_path)?;
        let metadata = fs::metadata(&canonical_path)?;
        let modified_nanoseconds = metadata
            .modified()?
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since_epoch| since_epoch.as_nanos());
        Ok(Self {
            canonical_path,
            length_in_bytes: metadata.len(),
            modified_nanoseconds,
        })
    }

    /// The name of the proxy in the cache: a hash of the fingerprint and of the
    /// proxy format version, as 16 hexadecimal digits.
    fn cache_key(&self) -> String {
        let mut hash = FnvHash::new();
        hash.write(&PROXY_FORMAT_VERSION.to_le_bytes());
        hash.write(self.canonical_path.to_string_lossy().as_bytes());
        hash.write(&[0]);
        hash.write(&self.length_in_bytes.to_le_bytes());
        hash.write(&self.modified_nanoseconds.to_le_bytes());
        format!("{:016x}", hash.finish())
    }
}

/// 64-bit FNV-1a: stable across Rust versions and machines, unlike `std`'s hasher.
struct FnvHash(u64);

impl FnvHash {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    fn new() -> Self {
        Self(Self::OFFSET_BASIS)
    }

    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.0 = (self.0 ^ u64::from(byte)).wrapping_mul(Self::PRIME);
        }
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

/// What is written next to each proxy, and checked before it is used.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProxyDescription {
    format_version: u32,
    source: SourceFingerprint,
    width_in_pixels: u32,
    height_in_pixels: u32,
}

/// A proxy ready to use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadyProxy {
    /// The source, as the project names it.
    pub source_path: PathBuf,
    /// The source's version the proxy was made from.
    pub source_fingerprint: SourceFingerprint,
    /// The proxy file.
    pub proxy_path: PathBuf,
    /// The size of the proxy's pictures.
    pub picture_size: PictureSize,
}

/// How far making a proxy has gone.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProxyProgress {
    /// Source time of the last frame written.
    pub done: TimeSpan,
    /// Length of the source, when known.
    pub total: Option<TimeSpan>,
}

impl ProxyProgress {
    /// Between 0 and 1 (0 while the length is unknown).
    pub fn fraction(&self) -> f64 {
        match self.total {
            Some(total) if !total.is_zero() => self.done.fraction_of(total).clamp(0.0, 1.0),
            _ => 0.0,
        }
    }
}

/// Why a proxy could not be made.
#[derive(Debug, thiserror::Error)]
pub enum ProxyError {
    /// The source could not be decoded or the proxy written.
    #[error(transparent)]
    Media(#[from] MediaError),
    /// The cache folder or the source could not be read or written.
    #[error("{action}: {source}")]
    Disk {
        /// What was being done.
        action: String,
        /// What the system said.
        source: io::Error,
    },
    /// The source changed while its proxy was being made.
    #[error("\"{}\" changed while its proxy was being made", file_path.display())]
    SourceChanged {
        /// The source.
        file_path: PathBuf,
    },
    /// The file written does not read back as expected.
    #[error("the proxy of \"{}\" does not read back as expected: {reason}", file_path.display())]
    UnreadableProxy {
        /// The source.
        file_path: PathBuf,
        /// What is wrong.
        reason: String,
    },
}

fn disk_error(action: String) -> impl FnOnce(io::Error) -> ProxyError {
    move |source| ProxyError::Disk { action, source }
}

/// How making a proxy ended, when it did not fail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProxyMakingOutcome {
    /// The proxy is in the cache.
    Ready(ReadyProxy),
    /// Stopped on request; nothing was kept.
    Cancelled,
}

/// The folder of proxies: `<key>.mp4`, and `<key>.json` describing its source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyCache {
    folder: PathBuf,
}

/// File name endings in the cache.
const PROXY_EXTENSION: &str = "mp4";
const DESCRIPTION_EXTENSION: &str = "json";
const UNFINISHED_SUFFIX: &str = ".making";

impl ProxyCache {
    /// The cache in `folder` (created when the first proxy is made).
    pub fn new(folder: PathBuf) -> Self {
        Self { folder }
    }

    /// Where the proxies are.
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    fn proxy_path_for(&self, key: &str) -> PathBuf {
        self.folder.join(format!("{key}.{PROXY_EXTENSION}"))
    }

    fn description_path_for(&self, key: &str) -> PathBuf {
        self.folder.join(format!("{key}.{DESCRIPTION_EXTENSION}"))
    }

    /// The proxy of `source_path` as the file is now, if the cache has one.
    /// Finding it marks it as used (the cache trims the least recently used).
    pub fn ready_proxy_of(&self, source_path: &Path) -> Option<ReadyProxy> {
        let fingerprint = SourceFingerprint::of(source_path).ok()?;
        let key = fingerprint.cache_key();
        let proxy_path = self.proxy_path_for(&key);
        let description_path = self.description_path_for(&key);
        let description: ProxyDescription =
            serde_json::from_slice(&fs::read(&description_path).ok()?).ok()?;
        let is_complete = proxy_path.is_file()
            && description.format_version == PROXY_FORMAT_VERSION
            && description.source == fingerprint;
        if !is_complete {
            return None;
        }
        mark_as_used(&description_path);
        Some(ReadyProxy {
            source_path: source_path.to_path_buf(),
            source_fingerprint: fingerprint,
            proxy_path,
            picture_size: PictureSize {
                width_in_pixels: description.width_in_pixels,
                height_in_pixels: description.height_in_pixels,
            },
        })
    }

    /// Makes the proxy of the video `source`, reporting progress to `on_progress`,
    /// which stops the work by returning [`ControlFlow::Break`]. The proxy is
    /// written under a temporary name and only takes its real name once
    /// complete and checked.
    ///
    /// # Errors
    ///
    /// See [`ProxyError`].
    pub fn make_proxy(
        &self,
        backend: &dyn MediaBackend,
        source: &MediaAssetDescription,
        mut on_progress: impl FnMut(ProxyProgress) -> ControlFlow<()>,
    ) -> Result<ProxyMakingOutcome, ProxyError> {
        let source_path = source.file_path.as_path();
        let picture_size = source
            .picture_size
            .filter(|_| source.kind == MediaKind::Video)
            .ok_or_else(|| MediaError::NoVideoStream {
                file_path: source_path.to_path_buf(),
            })?;
        let fingerprint = SourceFingerprint::of(source_path)
            .map_err(disk_error(format!("reading \"{}\"", source_path.display())))?;
        let key = fingerprint.cache_key();
        fs::create_dir_all(&self.folder).map_err(disk_error(format!(
            "creating the proxy folder \"{}\"",
            self.folder.display()
        )))?;
        let proxy_size = proxy_picture_size_for(picture_size);
        let unfinished_path = self
            .folder
            .join(format!("{key}.{}{UNFINISHED_SUFFIX}", std::process::id()));
        let settings = VideoProxySettings {
            source_path: source_path.to_path_buf(),
            output_path: unfinished_path.clone(),
            picture_size: proxy_size,
            frames_per_keyframe: PROXY_FRAMES_PER_KEYFRAME,
        };
        let total = source.duration;
        let made = backend.make_video_proxy(&settings, &mut |source_time: Timestamp| {
            on_progress(ProxyProgress {
                done: source_time.span_since(Timestamp::ZERO).unwrap_or_default(),
                total,
            })
        });
        let outcome = match made {
            Ok(VideoProxyOutcome::Finished) => self.keep_finished_proxy(
                backend,
                source_path,
                &fingerprint,
                &key,
                &unfinished_path,
                proxy_size,
            ),
            Ok(VideoProxyOutcome::Cancelled) => Ok(ProxyMakingOutcome::Cancelled),
            Err(error) => Err(error.into()),
        };
        // Whatever happened, the temporary file goes (it was renamed on success).
        let _ = fs::remove_file(&unfinished_path);
        outcome
    }

    /// Checks a finished proxy, then gives it its name and its description.
    fn keep_finished_proxy(
        &self,
        backend: &dyn MediaBackend,
        source_path: &Path,
        fingerprint: &SourceFingerprint,
        key: &str,
        unfinished_path: &Path,
        proxy_size: PictureSize,
    ) -> Result<ProxyMakingOutcome, ProxyError> {
        let still_the_same =
            SourceFingerprint::of(source_path).is_ok_and(|now| now == *fingerprint);
        if !still_the_same {
            return Err(ProxyError::SourceChanged {
                file_path: source_path.to_path_buf(),
            });
        }
        let written =
            backend
                .probe(unfinished_path)
                .map_err(|error| ProxyError::UnreadableProxy {
                    file_path: source_path.to_path_buf(),
                    reason: error.to_string(),
                })?;
        if written.picture_size != Some(proxy_size) {
            return Err(ProxyError::UnreadableProxy {
                file_path: source_path.to_path_buf(),
                reason: format!(
                    "its pictures are {:?} instead of {proxy_size:?}",
                    written.picture_size
                ),
            });
        }
        let description = ProxyDescription {
            format_version: PROXY_FORMAT_VERSION,
            source: fingerprint.clone(),
            width_in_pixels: proxy_size.width_in_pixels,
            height_in_pixels: proxy_size.height_in_pixels,
        };
        let proxy_path = self.proxy_path_for(key);
        fs::rename(unfinished_path, &proxy_path).map_err(disk_error(format!(
            "naming the proxy \"{}\"",
            proxy_path.display()
        )))?;
        write_description(&self.description_path_for(key), &description)?;
        Ok(ProxyMakingOutcome::Ready(ReadyProxy {
            source_path: source_path.to_path_buf(),
            source_fingerprint: fingerprint.clone(),
            proxy_path,
            picture_size: proxy_size,
        }))
    }

    /// The space the cache takes on disk.
    pub fn size_in_bytes(&self) -> u64 {
        self.entries()
            .iter()
            .map(|entry| entry.length_in_bytes)
            .sum()
    }

    /// Deletes every proxy except those in `kept_proxies`, and every unfinished
    /// one older than a day. Files in use elsewhere stay. Returns the space freed.
    pub fn remove_proxies_except(&self, kept_proxies: &HashSet<PathBuf>) -> u64 {
        self.remove_entries(self.entries(), kept_proxies)
    }

    /// Deletes the least recently used proxies (except `kept_proxies`) until
    /// the cache takes at most `largest_size_in_bytes`. Returns the space freed.
    pub fn trim_to_size(&self, largest_size_in_bytes: u64, kept_proxies: &HashSet<PathBuf>) -> u64 {
        let mut entries = self.entries();
        let mut size: u64 = entries.iter().map(|entry| entry.length_in_bytes).sum();
        entries.sort_by_key(|entry| entry.last_used);
        let mut removed = Vec::new();
        for entry in entries {
            if size <= largest_size_in_bytes {
                break;
            }
            if kept_proxies.contains(&entry.proxy_path) {
                continue;
            }
            size = size.saturating_sub(entry.length_in_bytes);
            removed.push(entry);
        }
        self.remove_entries(removed, kept_proxies)
    }

    fn remove_entries(&self, entries: Vec<CacheEntry>, kept_proxies: &HashSet<PathBuf>) -> u64 {
        let mut freed = 0;
        for entry in entries {
            if kept_proxies.contains(&entry.proxy_path) {
                continue;
            }
            // The proxy first: a description without its proxy is harmless.
            if fs::remove_file(&entry.proxy_path).is_ok() || !entry.proxy_path.exists() {
                freed += entry.length_in_bytes;
                if let Some(description_path) = &entry.description_path {
                    let _ = fs::remove_file(description_path);
                }
            }
        }
        freed
    }

    /// Every proxy, description and old unfinished file of the cache.
    fn entries(&self) -> Vec<CacheEntry> {
        let Ok(listing) = fs::read_dir(&self.folder) else {
            return Vec::new();
        };
        let mut entries = Vec::new();
        for item in listing.flatten() {
            let path = item.path();
            let Ok(metadata) = item.metadata() else {
                continue;
            };
            let name = item.file_name().to_string_lossy().into_owned();
            if name.ends_with(UNFINISHED_SUFFIX) {
                let is_old = metadata
                    .modified()
                    .ok()
                    .and_then(|modified| modified.elapsed().ok())
                    .is_some_and(|age| age > Duration::from_secs(24 * 60 * 60));
                if is_old {
                    entries.push(CacheEntry {
                        proxy_path: path,
                        description_path: None,
                        length_in_bytes: metadata.len(),
                        last_used: SystemTime::UNIX_EPOCH,
                    });
                }
                continue;
            }
            let Some(key) = name.strip_suffix(&format!(".{PROXY_EXTENSION}")) else {
                continue;
            };
            let description_path = self.description_path_for(key);
            let last_used = fs::metadata(&description_path)
                .and_then(|description| description.modified())
                .or_else(|_| metadata.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            let description_length = fs::metadata(&description_path).map_or(0, |m| m.len());
            entries.push(CacheEntry {
                proxy_path: path,
                description_path: Some(description_path),
                length_in_bytes: metadata.len() + description_length,
                last_used,
            });
        }
        entries
    }
}

/// One proxy in the cache folder.
struct CacheEntry {
    proxy_path: PathBuf,
    description_path: Option<PathBuf>,
    length_in_bytes: u64,
    last_used: SystemTime,
}

/// Writes a proxy's description atomically (a temporary file, then a rename).
fn write_description(
    description_path: &Path,
    description: &ProxyDescription,
) -> Result<(), ProxyError> {
    let text = serde_json::to_vec_pretty(description).map_err(|error| ProxyError::Disk {
        action: "describing the proxy".to_string(),
        source: io::Error::other(error),
    })?;
    let temporary_path = description_path.with_extension(format!(
        "{DESCRIPTION_EXTENSION}.{}{UNFINISHED_SUFFIX}",
        std::process::id()
    ));
    fs::write(&temporary_path, text).map_err(disk_error(format!(
        "writing \"{}\"",
        temporary_path.display()
    )))?;
    fs::rename(&temporary_path, description_path).map_err(|error| {
        let _ = fs::remove_file(&temporary_path);
        ProxyError::Disk {
            action: format!("writing \"{}\"", description_path.display()),
            source: error,
        }
    })
}

/// Marks a proxy as just used, for trimming. Failing changes nothing.
fn mark_as_used(description_path: &Path) {
    if let Ok(file) = fs::File::options().write(true).open(description_path) {
        let _ = file.set_modified(SystemTime::now());
    }
}

/// A proxy known to the preview, with when its source was last checked.
#[derive(Debug, Clone)]
struct RegisteredProxy {
    proxy: ReadyProxy,
    checked_at: Instant,
}

/// The proxies the preview may use now, by source path, shared between the
/// thread that makes them and the threads that show pictures.
///
/// Each lookup re-checks, at most every two seconds, that the source has not
/// changed since its proxy was made; a changed source loses its proxy.
#[derive(Debug)]
pub struct ReadyProxies {
    proxies: Mutex<HashMap<PathBuf, RegisteredProxy>>,
    are_enabled: AtomicBool,
}

impl Default for ReadyProxies {
    fn default() -> Self {
        Self {
            proxies: Mutex::new(HashMap::new()),
            are_enabled: AtomicBool::new(true),
        }
    }
}

impl ReadyProxies {
    /// No proxy yet, and proxies used once there are some.
    pub fn new() -> Self {
        Self::default()
    }

    /// Lets the preview use `proxy` for its source.
    pub fn add(&self, proxy: ReadyProxy) {
        self.lock().insert(
            proxy.source_path.clone(),
            RegisteredProxy {
                proxy,
                checked_at: Instant::now(),
            },
        );
    }

    /// Stops using the proxy of `source_path`, if any.
    pub fn remove(&self, source_path: &Path) {
        self.lock().remove(source_path);
    }

    /// Stops using every proxy.
    pub fn clear(&self) {
        self.lock().clear();
    }

    /// Turns the use of proxies on or off (they stay listed).
    pub fn set_enabled(&self, are_enabled: bool) {
        self.are_enabled.store(are_enabled, Ordering::Relaxed);
    }

    /// Whether proxies are used.
    pub fn are_enabled(&self) -> bool {
        self.are_enabled.load(Ordering::Relaxed)
    }

    /// The proxy files listed, used or not.
    pub fn proxy_paths(&self) -> HashSet<PathBuf> {
        self.lock()
            .values()
            .map(|registered| registered.proxy.proxy_path.clone())
            .collect()
    }

    /// The proxy of `source_path`, if one is listed for it.
    pub fn listed_proxy_of(&self, source_path: &Path) -> Option<ReadyProxy> {
        self.lock()
            .get(source_path)
            .map(|registered| registered.proxy.clone())
    }

    /// The proxy to read instead of `source_path`, when proxies are on and the
    /// source is unchanged since its proxy was made.
    pub fn proxy_for(&self, source_path: &Path) -> Option<ReadyProxy> {
        if !self.are_enabled() {
            return None;
        }
        let mut proxies = self.lock();
        let registered = proxies.get_mut(source_path)?;
        if registered.checked_at.elapsed() >= SOURCE_CHECK_INTERVAL {
            let is_unchanged = SourceFingerprint::of(source_path)
                .is_ok_and(|now| now == registered.proxy.source_fingerprint);
            if !is_unchanged {
                proxies.remove(source_path);
                return None;
            }
            registered.checked_at = Instant::now();
        }
        Some(registered.proxy.clone())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<PathBuf, RegisteredProxy>> {
        self.proxies
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// `region`, given in the pixels of a picture of `original_size`, moved to a
/// picture of `shown_size` (a proxy): the area, the blur radius, the pixel
/// blocks and the edge softness all scale, so the proxy shows the same result.
pub fn hidden_region_for_picture_size(
    region: &HiddenRegion,
    original_size: PictureSize,
    shown_size: PictureSize,
) -> HiddenRegion {
    if original_size == shown_size
        || original_size.width_in_pixels == 0
        || original_size.height_in_pixels == 0
    {
        return *region;
    }
    let horizontal = shown_size.width_in_pixels as f32 / original_size.width_in_pixels as f32;
    let vertical = shown_size.height_in_pixels as f32 / original_size.height_in_pixels as f32;
    let average = (horizontal + vertical) / 2.0;
    let style = match region.style {
        BlurStyle::GaussianBlur { radius_in_pixels } => BlurStyle::GaussianBlur {
            radius_in_pixels: radius_in_pixels * average,
        },
        BlurStyle::Pixelate {
            block_size_in_pixels,
        } => BlurStyle::Pixelate {
            block_size_in_pixels: ((block_size_in_pixels as f32 * average).round() as u32).max(1),
        },
        BlurStyle::SolidFill { color } => BlurStyle::SolidFill { color },
    };
    HiddenRegion {
        area: PixelRegion {
            left_in_pixels: region.area.left_in_pixels * horizontal,
            top_in_pixels: region.area.top_in_pixels * vertical,
            width_in_pixels: region.area.width_in_pixels * horizontal,
            height_in_pixels: region.area.height_in_pixels * vertical,
        },
        style,
        shape: region.shape,
        edge_softness_in_pixels: region.edge_softness_in_pixels * average,
    }
}

#[cfg(test)]
mod tests {
    use kadr_core::effect::BlurShape;
    use kadr_core::time::FrameRate;
    use kadr_media::ChromaSubsampling;

    use super::*;

    fn size(width_in_pixels: u32, height_in_pixels: u32) -> PictureSize {
        PictureSize {
            width_in_pixels,
            height_in_pixels,
        }
    }

    fn video(
        codec_name: &str,
        picture_size: PictureSize,
        frames_per_second: u32,
    ) -> VideoStreamDetails {
        VideoStreamDetails {
            codec_name: codec_name.to_string(),
            picture_size,
            frame_rate: Some(FrameRate::new(frames_per_second, 1).unwrap()),
            bits_per_sample: 8,
            chroma_subsampling: ChromaSubsampling::Yuv420,
            bit_rate_in_bits_per_second: Some(20_000_000),
        }
    }

    #[test]
    fn pictures_larger_than_1080p_get_a_proxy() {
        for picture_size in [
            size(3840, 2160),
            size(2160, 3840),
            size(2560, 1440),
            size(2704, 1520),
        ] {
            assert_eq!(
                proxy_need_of(&video("h264", picture_size, 30)),
                Some(ProxyReason::LargePicture),
                "{picture_size:?}"
            );
        }
        for picture_size in [size(1920, 1080), size(1080, 1920), size(1280, 720)] {
            assert_eq!(proxy_need_of(&video("h264", picture_size, 30)), None);
        }
    }

    #[test]
    fn slow_codecs_frame_rates_and_formats_get_a_proxy() {
        let full_hd = size(1920, 1080);
        assert_eq!(proxy_need_of(&video("h264", full_hd, 60)), None);
        assert_eq!(
            proxy_need_of(&video("h264", full_hd, 120)),
            Some(ProxyReason::SlowToDecode)
        );
        assert_eq!(proxy_need_of(&video("hevc", full_hd, 30)), None);
        assert_eq!(
            proxy_need_of(&video("hevc", full_hd, 60)),
            Some(ProxyReason::SlowToDecode)
        );
        let ten_bit_hevc = VideoStreamDetails {
            bits_per_sample: 10,
            ..video("hevc", full_hd, 30)
        };
        assert_eq!(
            proxy_need_of(&ten_bit_hevc),
            Some(ProxyReason::SlowToDecode)
        );
        let ten_bit_hevc_720p = VideoStreamDetails {
            bits_per_sample: 10,
            ..video("hevc", size(1280, 720), 30)
        };
        assert_eq!(proxy_need_of(&ten_bit_hevc_720p), None);
    }

    #[test]
    fn very_high_bit_rates_get_a_proxy() {
        let intermediate = VideoStreamDetails {
            bit_rate_in_bits_per_second: Some(220_000_000),
            ..video("prores", size(1920, 1080), 30)
        };
        assert_eq!(proxy_need_of(&intermediate), Some(ProxyReason::HighBitRate));
    }

    #[test]
    fn proxies_fit_inside_1080p_with_even_sides() {
        assert_eq!(proxy_picture_size_for(size(3840, 2160)), size(1920, 1080));
        assert_eq!(proxy_picture_size_for(size(2160, 3840)), size(1080, 1920));
        assert_eq!(proxy_picture_size_for(size(4096, 2160)), size(1920, 1012));
        assert_eq!(proxy_picture_size_for(size(1920, 1080)), size(1920, 1080));
        assert_eq!(proxy_picture_size_for(size(1279, 719)), size(1278, 718));
    }

    #[test]
    fn fingerprints_change_with_the_file() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("video.mp4");
        fs::write(&path, b"first").unwrap();
        let first = SourceFingerprint::of(&path).unwrap();
        assert_eq!(
            SourceFingerprint::of(&path).unwrap().cache_key(),
            first.cache_key()
        );
        fs::write(&path, b"second, longer").unwrap();
        let second = SourceFingerprint::of(&path).unwrap();
        assert_ne!(second.cache_key(), first.cache_key());
        let moved = folder.path().join("moved.mp4");
        fs::rename(&path, &moved).unwrap();
        assert_ne!(
            SourceFingerprint::of(&moved).unwrap().cache_key(),
            second.cache_key()
        );
    }

    #[test]
    fn a_proxy_whose_description_does_not_match_is_not_used() {
        let folder = tempfile::tempdir().unwrap();
        let source = folder.path().join("video.mp4");
        fs::write(&source, b"pictures").unwrap();
        let cache = ProxyCache::new(folder.path().join("proxies"));
        fs::create_dir_all(cache.folder()).unwrap();
        let fingerprint = SourceFingerprint::of(&source).unwrap();
        let key = fingerprint.cache_key();
        fs::write(cache.proxy_path_for(&key), b"proxy").unwrap();
        let description = ProxyDescription {
            format_version: PROXY_FORMAT_VERSION,
            source: fingerprint,
            width_in_pixels: 1920,
            height_in_pixels: 1080,
        };
        write_description(&cache.description_path_for(&key), &description).unwrap();
        assert_eq!(
            cache.ready_proxy_of(&source).unwrap().picture_size,
            size(1920, 1080)
        );

        let older_format = ProxyDescription {
            format_version: PROXY_FORMAT_VERSION + 1,
            ..description
        };
        write_description(&cache.description_path_for(&key), &older_format).unwrap();
        assert_eq!(cache.ready_proxy_of(&source), None);
    }

    #[test]
    fn trimming_removes_the_least_recently_used_proxies_first() {
        let folder = tempfile::tempdir().unwrap();
        let cache = ProxyCache::new(folder.path().to_path_buf());
        let now = SystemTime::now();
        for (index, key) in ["old", "middle", "new"].into_iter().enumerate() {
            fs::write(cache.proxy_path_for(key), vec![0; 100]).unwrap();
            let description_path = cache.description_path_for(key);
            fs::write(&description_path, b"{}").unwrap();
            let file = fs::File::options()
                .write(true)
                .open(&description_path)
                .unwrap();
            file.set_modified(now - Duration::from_secs(1_000 - 100 * index as u64))
                .unwrap();
        }
        let kept = HashSet::from([cache.proxy_path_for("old")]);
        let freed = cache.trim_to_size(150, &kept);
        assert_eq!(freed, 102 * 2);
        assert!(cache.proxy_path_for("old").exists());
        assert!(!cache.proxy_path_for("middle").exists());
        assert!(!cache.proxy_path_for("new").exists());
        assert_eq!(cache.size_in_bytes(), 102);
        assert_eq!(cache.remove_proxies_except(&HashSet::new()), 102);
        assert_eq!(cache.size_in_bytes(), 0);
    }

    #[test]
    fn listed_proxies_are_dropped_when_their_source_changes() {
        let folder = tempfile::tempdir().unwrap();
        let source = folder.path().join("video.mp4");
        fs::write(&source, b"pictures").unwrap();
        let proxies = ReadyProxies::new();
        let proxy = ReadyProxy {
            source_path: source.clone(),
            source_fingerprint: SourceFingerprint::of(&source).unwrap(),
            proxy_path: folder.path().join("proxy.mp4"),
            picture_size: size(960, 540),
        };
        proxies.add(proxy.clone());
        assert_eq!(proxies.proxy_for(&source), Some(proxy.clone()));
        proxies.set_enabled(false);
        assert_eq!(proxies.proxy_for(&source), None);
        proxies.set_enabled(true);

        fs::write(&source, b"other pictures").unwrap();
        // Pretend the last check was long ago.
        proxies.lock().get_mut(&source).unwrap().checked_at =
            Instant::now() - SOURCE_CHECK_INTERVAL;
        assert_eq!(proxies.proxy_for(&source), None);
        assert_eq!(proxies.listed_proxy_of(&source), None);
    }

    #[test]
    fn hidden_regions_scale_to_the_proxy() {
        let region = HiddenRegion {
            area: PixelRegion {
                left_in_pixels: 1000.0,
                top_in_pixels: 400.0,
                width_in_pixels: 200.0,
                height_in_pixels: 300.0,
            },
            style: BlurStyle::Pixelate {
                block_size_in_pixels: 25,
            },
            shape: BlurShape::Ellipse,
            edge_softness_in_pixels: 8.0,
        };
        let scaled = hidden_region_for_picture_size(&region, size(3840, 2160), size(1920, 1080));
        assert_eq!(scaled.area.left_in_pixels, 500.0);
        assert_eq!(scaled.area.top_in_pixels, 200.0);
        assert_eq!(scaled.area.width_in_pixels, 100.0);
        assert_eq!(scaled.area.height_in_pixels, 150.0);
        assert_eq!(
            scaled.style,
            BlurStyle::Pixelate {
                block_size_in_pixels: 13
            }
        );
        assert_eq!(scaled.edge_softness_in_pixels, 4.0);
        let unchanged = hidden_region_for_picture_size(&region, size(1920, 1080), size(1920, 1080));
        assert_eq!(unchanged, region);
    }
}
