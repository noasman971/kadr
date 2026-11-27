//! Measures preview frame times on one video, without and then with its proxy
//! (decision record 0055), and prints a Markdown table.
//!
//! ```powershell
//! cargo run --release -p kadr-engine --example preview_frame_times -- video.mp4
//! cargo run --release -p kadr-engine --example preview_frame_times -- video.mp4 --canvas 1920x1080 --size 1280x720
//! ```
//!
//! - **Decoding**: the video source alone (`frame_at`), frames in order, then at
//!   random times (jumps).
//! - **Playback**: preview frames in order at the project's frame rate.
//! - **Scrubbing**: preview frames at random times, as when dragging the playhead.
//!
//! The project shows the video alone on a canvas of its own size (what the app's
//! automatic format does), or of `--canvas`. Preview frames fit inside `--size`
//! (1280 × 720 by default, a preview panel on a laptop screen). The proxy is made
//! in a temporary folder, deleted at the end.
//!
//! `--decoding-only` times only the decoding of the file given, opened as an
//! original and as a proxy (to compare proxy formats made by hand):
//!
//! ```powershell
//! cargo run --release -p kadr-engine --example preview_frame_times -- proxy.mp4 --decoding-only
//! ```

use std::error::Error;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use kadr_core::media_asset::PictureSize;
use kadr_core::project::{Project, ProjectSettings};
use kadr_core::time::{TimeSpan, Timestamp};
use kadr_engine::RendererPreference;
use kadr_engine::preview::PreviewRenderer;
use kadr_engine::proxies::{
    ProxyCache, ProxyMakingOutcome, ReadyProxies, proxy_need_of, proxy_picture_size_for,
};
use kadr_engine::quick_assembly::assemble_project_from_files;
use kadr_media::{FfmpegBackend, MediaBackend};

/// Frames rendered in order, after a warm-up.
const PLAYBACK_FRAME_COUNT: u64 = 90;
/// Random positions asked for.
const SCRUB_POSITION_COUNT: usize = 40;
/// Frames rendered before measuring, so caches and the GPU are warm.
const WARM_UP_FRAME_COUNT: u64 = 5;

fn main() -> Result<(), Box<dyn Error>> {
    let options = Options::from_arguments()?;
    let backend = FfmpegBackend::new()?;
    if options.only_decoding {
        return print_decoding_times(&backend, &options.video);
    }
    let description = backend.probe(&options.video)?;
    let details = backend.describe_video_stream(&options.video)?;
    let (Some(picture_size), Some(duration)) = (description.picture_size, description.duration)
    else {
        return Err("the file is not a video with a known length".into());
    };
    println!(
        "{}: {} {}×{}, {} bits, {:?}, {:.1} Mbit/s, {:?} fps",
        options.video.display(),
        details.codec_name,
        picture_size.width_in_pixels,
        picture_size.height_in_pixels,
        details.bits_per_sample,
        details.chroma_subsampling,
        details.bit_rate_in_bits_per_second.unwrap_or(0) as f64 / 1e6,
        details.frame_rate.map(|rate| rate.as_frames_per_second()),
    );
    println!("Proxy rule: {:?}", proxy_need_of(&details));

    let canvas_size = options.canvas_size.unwrap_or(picture_size);
    let project = assemble_project_from_files(
        &backend,
        std::slice::from_ref(&options.video),
        ProjectSettings {
            canvas_size,
            ..ProjectSettings::landscape_1080p()
        },
        TimeSpan::from_whole_seconds(3),
    )?;
    let positions = Positions::for_project(&project, duration);

    let mut rows = vec![measure(
        "Original", &backend, &project, &options, &positions, None,
    )?];

    let cache_folder = tempfile::tempdir()?;
    let cache = ProxyCache::new(cache_folder.path().to_path_buf());
    let started = Instant::now();
    let outcome = cache.make_proxy(&backend, &description, |_| ControlFlow::Continue(()))?;
    let making_time = started.elapsed();
    let ProxyMakingOutcome::Ready(proxy) = outcome else {
        return Err("the proxy was cancelled".into());
    };
    let proxy_file_size = std::fs::metadata(&proxy.proxy_path)?.len();
    println!(
        "Proxy {}×{}: made in {:.1} s ({:.2}× real time), {:.1} MB ({:.1} Mbit/s)",
        proxy.picture_size.width_in_pixels,
        proxy.picture_size.height_in_pixels,
        making_time.as_secs_f64(),
        duration.as_seconds() / making_time.as_secs_f64(),
        proxy_file_size as f64 / 1e6,
        proxy_file_size as f64 * 8.0 / duration.as_seconds() / 1e6,
    );
    assert_eq!(
        proxy.picture_size,
        proxy_picture_size_for(picture_size),
        "the proxy has its planned size"
    );
    let ready_proxies = Arc::new(ReadyProxies::new());
    ready_proxies.add(proxy);
    rows.push(measure(
        "Proxy",
        &backend,
        &project,
        &options,
        &positions,
        Some(ready_proxies),
    )?);

    println!();
    println!(
        "| Pictures | Decoding in order | Decoding, jumps | Playback (preview) | Scrubbing (preview) |"
    );
    println!("|---|---|---|---|---|");
    for row in rows {
        println!(
            "| {} | {} | {} | {} | {} |",
            row.label, row.decoding_in_order, row.decoding_jumps, row.playback, row.scrubbing
        );
    }
    Ok(())
}

/// Command-line options.
struct Options {
    video: PathBuf,
    canvas_size: Option<PictureSize>,
    preview_size: PictureSize,
    renderer: RendererPreference,
    only_decoding: bool,
}

impl Options {
    fn from_arguments() -> Result<Self, Box<dyn Error>> {
        let mut arguments = std::env::args().skip(1);
        let mut video = None;
        let mut canvas_size = None;
        let mut preview_size = PictureSize {
            width_in_pixels: 1280,
            height_in_pixels: 720,
        };
        let mut renderer = RendererPreference::GpuIfAvailable;
        let mut only_decoding = false;
        while let Some(argument) = arguments.next() {
            match argument.as_str() {
                "--canvas" => canvas_size = Some(parse_size(arguments.next())?),
                "--size" => preview_size = parse_size(arguments.next())?,
                "--cpu" => renderer = RendererPreference::CpuOnly,
                "--decoding-only" => only_decoding = true,
                _ => video = Some(PathBuf::from(argument)),
            }
        }
        Ok(Self {
            video: video
                .ok_or("usage: preview_frame_times <video> [--canvas WxH] [--size WxH] [--cpu]")?,
            canvas_size,
            preview_size,
            renderer,
            only_decoding,
        })
    }
}

fn parse_size(text: Option<String>) -> Result<PictureSize, Box<dyn Error>> {
    let text = text.ok_or("a size is written WIDTHxHEIGHT")?;
    let (width, height) = text
        .split_once('x')
        .ok_or("a size is written WIDTHxHEIGHT")?;
    Ok(PictureSize {
        width_in_pixels: width.parse()?,
        height_in_pixels: height.parse()?,
    })
}

/// The times measured, the same for every row.
struct Positions {
    /// Timeline (and source) times played in order.
    in_order: Vec<Timestamp>,
    /// Timeline (and source) times in a random order.
    random: Vec<Timestamp>,
}

impl Positions {
    fn for_project(project: &Project, duration: TimeSpan) -> Self {
        let frame_rate = project.settings().frame_rate;
        let first_frame = frame_rate.frame_index_at(Timestamp::from_milliseconds(1_000));
        let in_order = (first_frame..first_frame + WARM_UP_FRAME_COUNT + PLAYBACK_FRAME_COUNT)
            .map(|frame_index| frame_rate.timestamp_of_frame(frame_index))
            .collect();
        // A fixed pseudo-random sequence (64-bit linear congruential generator),
        // so every run and every row asks for the same times.
        let mut state: u64 = 0x2545_f491_4f6c_dd1d;
        let last_frame = frame_rate.frame_index_at(Timestamp::ZERO + duration).max(1) - 1;
        let random = (0..SCRUB_POSITION_COUNT)
            .map(|_| {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                frame_rate.timestamp_of_frame((state >> 33) % last_frame)
            })
            .collect();
        Self { in_order, random }
    }
}

/// Median and 95th percentile of some durations, as table text.
fn summary_of(mut durations: Vec<Duration>) -> String {
    durations.sort();
    let at_fraction = |fraction: f64| {
        let index = ((durations.len() - 1) as f64 * fraction).round() as usize;
        durations[index].as_secs_f64() * 1_000.0
    };
    format!(
        "{:.1} ms (95 %: {:.1} ms)",
        at_fraction(0.5),
        at_fraction(0.95)
    )
}

/// One row of the table.
struct Row {
    label: &'static str,
    decoding_in_order: String,
    decoding_jumps: String,
    playback: String,
    scrubbing: String,
}

fn measure(
    label: &'static str,
    backend: &FfmpegBackend,
    project: &Project,
    options: &Options,
    positions: &Positions,
    proxies: Option<Arc<ReadyProxies>>,
) -> Result<Row, Box<dyn Error>> {
    let picture_file: PathBuf = match &proxies {
        Some(proxies) => proxies
            .proxy_for(&options.video)
            .map(|proxy| proxy.proxy_path)
            .ok_or("no proxy listed")?,
        None => options.video.clone(),
    };
    let opens_proxy = proxies.is_some();
    let (decoding_in_order, decoding_jumps) =
        time_decoding(backend, &picture_file, opens_proxy, positions)?;

    let mut renderer = PreviewRenderer::new(options.renderer)?;
    if let Some(proxies) = proxies {
        renderer.use_proxies(proxies);
    }
    let mut render_at = |time: Timestamp| -> Result<Duration, Box<dyn Error>> {
        let started = Instant::now();
        renderer.render_frame(project, backend, time, options.preview_size)?;
        Ok(started.elapsed())
    };
    let mut playback = Vec::new();
    for (index, &time) in positions.in_order.iter().enumerate() {
        let elapsed = render_at(time)?;
        if index as u64 >= WARM_UP_FRAME_COUNT {
            playback.push(elapsed);
        }
    }
    let scrubbing = positions
        .random
        .iter()
        .map(|&time| render_at(time))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Row {
        label,
        decoding_in_order: summary_of(decoding_in_order),
        decoding_jumps: summary_of(decoding_jumps),
        playback: summary_of(playback),
        scrubbing: summary_of(scrubbing),
    })
}

/// Times `frame_at` alone on `file`: in order, then at random times.
fn time_decoding(
    backend: &FfmpegBackend,
    file: &Path,
    opens_proxy: bool,
    positions: &Positions,
) -> Result<(Vec<Duration>, Vec<Duration>), Box<dyn Error>> {
    let mut source = if opens_proxy {
        backend.open_video_proxy(file)?
    } else {
        backend.open_video_source(file)?
    };
    let mut in_order = Vec::new();
    for (index, &time) in positions.in_order.iter().enumerate() {
        let started = Instant::now();
        source.frame_at(time)?;
        if index as u64 >= WARM_UP_FRAME_COUNT {
            in_order.push(started.elapsed());
        }
    }
    let mut jumps = Vec::new();
    for &time in &positions.random {
        let started = Instant::now();
        source.frame_at(time)?;
        jumps.push(started.elapsed());
    }
    Ok((in_order, jumps))
}

/// `--decoding-only`: decoding times of `file`, opened both ways.
fn print_decoding_times(backend: &FfmpegBackend, file: &Path) -> Result<(), Box<dyn Error>> {
    let description = backend.probe(file)?;
    let duration = description.duration.ok_or("the file has no length")?;
    let project = Project::new(ProjectSettings::landscape_1080p());
    let positions = Positions::for_project(&project, duration);
    let size_in_bytes = std::fs::metadata(file)?.len();
    println!(
        "{}: {:.1} MB ({:.1} Mbit/s)",
        file.display(),
        size_in_bytes as f64 / 1e6,
        size_in_bytes as f64 * 8.0 / duration.as_seconds() / 1e6
    );
    for (label, opens_proxy) in [("frame threads", false), ("slice threads", true)] {
        let (in_order, jumps) = time_decoding(backend, file, opens_proxy, &positions)?;
        println!(
            "| {label} | {} | {} |",
            summary_of(in_order),
            summary_of(jumps)
        );
    }
    Ok(())
}
