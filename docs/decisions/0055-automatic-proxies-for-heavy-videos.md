# 0055. Automatic proxies for heavy videos: 1080p H.264 for quick jumps, in a cache outside projects, for the preview only

- Status: Accepted
- Date: 2026-11-27

## Context

The specification asks for "automatic proxies for heavy 4K videos" (phase 8). A 4K
phone or camera video decodes well enough in order, but the preview jumps all the
time: scrubbing, clicking the ruler, the first frame after an edit. Compressed video
can only be entered at keyframes, which such files place one or two seconds apart,
and FFmpeg's frame-threaded decoder adds its own delay to every jump. Measured on the
owner's laptop (Intel Core Ultra 9 185H, Intel Arc) with
`scripts/generate-proxy-benchmark.ps1`, a jump in a 4K H.264 file at 60 Mbit/s took
**0.7 to 1.2 s**: dragging the playhead showed a frame now and then.

The usual answer is a *proxy*: a lighter copy the editor shows while editing, and an
export from the originals. Questions to answer:

- which videos get one, automatically;
- the proxy's format, for decoding and jumping fast;
- where proxies live, and how a changed or moved source never gets an old one;
- what reads them, and how positions, crops and blur regions stay in the original's
  pixels;
- whether anything goes into the `.kadr` file;
- how the app shows it, stops it, turns it off and frees the space.

## Options considered

1. **The rule.**
   1. Size only (above 1920 × 1080): simple, and what "4K" means to people. But a
      1080p60 HEVC 10-bit phone video or 1080p120 slow motion is as heavy.
   2. Size, plus an estimated decoding cost (pixels per second, times the codec's
      cost, times the cost of converting pictures that are not 8-bit 4:2:0), plus
      the bit rate. Explainable with three numbers.
   3. Measuring each file's decoding speed at import: exact, but slow and different
      on every run of a busy laptop.
2. **The proxy's codec.**
   1. ProRes Proxy or DNxHR LB, what professional editors use: intra-only, but FFmpeg's
      encoders are slow, the files five to ten times larger, and Android cannot
      decode them.
   2. MJPEG: intra-only and simple, but big files and full-range colours.
   3. H.264 with libx264: every platform decodes it, `fastdecode` drops the costly
      parts (CABAC, deblocking), and the keyframe interval and slices are ours to
      choose.
3. **The proxy's size**: 1280 × 720 (smaller files) or 1920 × 1080 (sharp in any
   preview panel, and at the size of a 1080p canvas).
4. **Where proxies live**: next to the source (writes into the user's folders, and
   travels with them), next to the project (the same, and several projects share
   videos), or in the app's cache folder (per computer, outside OneDrive).
5. **The key of a proxy**: the source's content hash (exact, but reading gigabytes),
   or its full path, length and modification time (instant; a moved file gets a new
   proxy).
6. **What uses proxies**: the preview only, or also thumbnails, Claude's pictures,
   the blur analysis and the export.
7. **The project file**: a proxy path per media asset, or nothing.

## Decision

- **The rule is option 1.2** (`kadr_engine::proxies::proxy_need_of`). A video gets a
  proxy when:
  - its picture is larger than 1920 × 1080 either way round
    (`ProxyReason::LargePicture`);
  - or its estimated decoding work is more than 1080p H.264 at 60 fps
    (`SlowToDecode`): pixels per second, × 2 for HEVC, VP9 and AV1, × 2 for pictures
    other than 8-bit 4:2:0 (10-bit, 4:2:2…), which are converted on every frame. So
    1080p30 HEVC 10-bit, 1080p60 HEVC and 1080p120 H.264 get one; 1080p30 HEVC and
    1080p60 H.264 do not;
  - or its bit rate is above 100 Mbit/s (`HighBitRate`).

  The desktop app applies it to every video noticed in a session (opened projects,
  imports, Claude's imports). The user can ask for a proxy anyway (*make again*).
- **The format is option 2.3 at 1920 × 1080 (option 3)**, written by
  `MediaBackend::make_video_proxy` (`kadr-media/src/ffmpeg/video_proxy.rs`):
  - pictures shrunk to fit inside 1920 × 1080 (1080 × 1920 upright) with even sides,
    by swscale (bicubic), as 8-bit 4:2:0;
  - libx264 `veryfast`, `tune=fastdecode`, CRF 23, **no B-frames**, **a keyframe at
    least every 5 frames**, **4 slices per frame**;
  - every source frame kept at its own time (variable frame rates included), with
    the times read exactly as the reader reads them, so `frame_at(t)` shows the same
    frame in the proxy and the original;
  - tagged with the colour matrix and range the original is read in;
  - no sound: the preview's sound comes from the original.

  Proxies are opened with `open_video_proxy`, which decodes a frame's slices in
  parallel (**slice threads**). Frame threads, kept for originals, decode one frame
  per core at once and hold back the first frame after a jump until a frame per
  thread is done: with them, even a proxy with frequent keyframes took about 50 ms a
  jump.
- **Proxies live in the app's cache folder** (option 4, third choice): on Windows
  `%LOCALAPPDATA%\org.kadr.desktop\proxies`. `ProxyCache` names each proxy from a
  64-bit FNV-1a hash (stable across Rust versions) of the format version, the
  source's canonical path, its length and its modification time (option 5, second
  choice), with a `.json` description repeating them, checked before use:
  - a source edited or replaced in place changes its length or time: a new proxy;
  - a moved or renamed source gets a new proxy (the old one ages out);
  - while the app runs, `ReadyProxies` checks the source again at most every two
    seconds, and drops a proxy whose source changed;
  - proxies are written under a temporary name, probed, then renamed: a stopped or
    failed proxy never looks complete;
  - after each new proxy, the cache keeps at most 10 GB, deleting the least recently
    used first (never one of the session's).
- **What uses proxies** (option 6): the preview and the thumbnails, through
  `ReadyProxies`. Only the preview's frame composer and the thumbnail maker are given
  it; the export builds its own composer without it, so it cannot read a proxy.
  Claude's pictures (`get_frame`, contact sheets) and the blur analysis read the
  originals: they give and use coordinates in the original's pixels and need its
  detail, and `kadr-mcp` in file mode has no cache. Waveforms and sound come from
  the originals too.
- **Coordinates stay the original's.** Positions are canvas pixels and crops are
  fractions, which do not depend on the picture's size; videos are fitted to the
  canvas whatever their size. Tracked blur regions (and their radius, blocks and
  softness) are in the original's pixels: the preview's composer scales them to the
  proxy's just before hiding them. The blur overlay maps the original's pixels, as
  before.
- **Nothing goes into the `.kadr` file** (option 7, second choice): projects stay
  portable (a proxy path would name another computer's cache), the cache can be
  deleted at any time, and the file format does not change.
- **The preview composes large canvases smaller.** With proxies, a 4K project's
  preview spent most of its time composing a 4K canvas and shrinking it. The
  preview now composes at the canvas's half (or quarter…) while that still covers
  the panel (`preview::composition_size_for`), scaling positions and text sizes:
  what an export at that size would draw.
- **The app** (`proxy_jobs.rs`): one worker thread, one proxy at a time, progress
  through `proxy-changed` events; stop, make again, the setting (*Proxies for heavy
  videos*, on by default, in `proxy-settings.json`; off, nothing is made or used)
  and *Delete the proxies* (the picture threads close their files first).

## Measurements

Owner's laptop (Intel Core Ultra 9 185H, Intel Arc), release build, the two 20 s
clips of `scripts/generate-proxy-benchmark.ps1`, measured with
`cargo run --release -p kadr-engine --example preview_frame_times -- <clip>`: a
project of the clip alone on a 3840 × 2160 canvas, preview frames fitting 1280 × 720.
Medians per frame. Three other Kadr sessions were compiling at the same time (the
processor was 25–90 % busy), so the 95th percentiles (not shown) are noisy and
single numbers move by up to 2× between runs; the ratios hold.

| 4K H.264, 60 Mbit/s, keyframe every 2 s | Decoding in order | A jump | Playback | Scrubbing |
|---|---|---|---|---|
| Original | 7.9 ms | 1 246 ms | 58.7 ms | 1 597 ms |
| Proxy | 2.5 ms | 14.8 ms | **6.4 ms** | **13.4 ms** |

| 4K HEVC Main 10, 51 Mbit/s, keyframe every 1 s | Decoding in order | A jump | Playback | Scrubbing |
|---|---|---|---|---|
| Original | 22.3 ms | 1 370 ms | 61.2 ms | 1 388 ms |
| Proxy | 1.7 ms | 8.9 ms | **6.2 ms** | **19.3 ms** |

- *Playback* and *Scrubbing* are whole preview frames (decoding, composing,
  shrinking), in order at 30 fps and at random times.
- Making the proxies took 17.3 s and 18.8 s for 20 s of video (about real time,
  mostly decoding the 4K original); they weigh 27.3 MB and 23.3 MB (about
  80 MB per minute).
- On a 1920 × 1080 canvas, the H.264 clip's proxy gave 13.7 ms per frame in playback
  and 26.7 ms scrubbing, against 69.9 ms and 1 172 ms from the original.
- In the app itself (debug build, same load), preview frames of the H.264 clip took
  292 ms scrubbing from the proxy against 1 535 ms from the original.

The proxy format was chosen by decoding 1080p copies of the H.264 clip made with
different settings (`--decoding-only`, medians, same busy laptop):

| Keyframes, slices | Size | Jump, frame threads | Jump, slice threads |
|---|---|---|---|
| every frame, 4 slices | 25.2 Mbit/s | 111 ms | 21 ms |
| **every 5 frames, 4 slices** | **10.9 Mbit/s** | 88 ms | **25 ms** |
| every 10 frames, 1 slice | 8.2 Mbit/s | 107 ms | 118 ms |
| every 10 frames, 4 slices | 8.3 Mbit/s | 119 ms | 48 ms |
| every 10 frames, 8 slices | 8.4 Mbit/s | 88 ms | 43 ms |
| every 30 frames, 4 slices | 6.1 Mbit/s | 106 ms | 72 ms |

Slice threads only help when the file has slices; a keyframe every 5 frames jumps
almost as fast as one on every frame, at less than half the size. Before the
preview composed large canvases smaller, a proxy on a 4K canvas still cost 59 ms per
frame in playback: drawing and shrinking the 4K canvas.

## Consequences

- Scrubbing a 4K video is fifty to a hundred times quicker once its proxy is ready,
  and playing it costs a tenth; until then, the preview reads the original.
- Making a proxy takes about as long as the video lasts (decoding the 4K original is
  most of it), in the background. Proxies take about 80 MB per minute.
- Exports are unchanged, byte for byte (tested): they never read a proxy.
- A preview frame from a proxy is softer than the original at full size on a 4K
  screen. Turning proxies off shows the originals.
- The rule's numbers are estimates: a file just under them may still be heavy on a
  slow computer. *Make a proxy* (after stopping one) covers that; a per-file button
  for any video could come later.
- Android (phase 9) has the same traits: `MediaCodec` can implement
  `make_video_proxy` and `open_video_proxy`, or keep the defaults (no proxies).
- A proxy stopped by the user is made again at the next opening of the project in a
  later session.
