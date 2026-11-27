<#
.SYNOPSIS
    Generates the heavy 4K videos used to measure preview frame times with and
    without proxies (decision record 0055).

.DESCRIPTION
    Two 20-second, 3840 x 2160, 30 fps clips with fine moving detail (a slow pan
    over a detailed Mandelbrot picture with film grain, so the encoders cannot
    cheat), sound included:
        4k_h264_60mbps.mp4    H.264 High, 8-bit 4:2:0, about 60 Mbit/s, a
                              keyframe every 2 s (like a camera or a phone)
        4k_hevc_10bit.mp4     HEVC Main 10, 10-bit 4:2:0, about 50 Mbit/s, a
                              keyframe every second (like an HDR phone video)
    They land in a temporary folder (about 270 MB in all), never in the
    repository. Delete the folder when done.

    Measure the preview on them (release build):
        cargo run --release -p kadr-engine --example preview_frame_times -- <video>

    Requires the `ffmpeg` command on the PATH, built with libx264 and libx265.

.PARAMETER OutputDirectory
    Where to write the clips. Defaults to kadr-proxy-benchmark in the temporary folder.

.EXAMPLE
    ./scripts/generate-proxy-benchmark.ps1
#>

param(
    [string] $OutputDirectory = (Join-Path ([System.IO.Path]::GetTempPath()) 'kadr-proxy-benchmark')
)

$ErrorActionPreference = 'Stop'

if (-not (Get-Command 'ffmpeg' -ErrorAction SilentlyContinue)) {
    throw "ffmpeg was not found on the PATH. Install it first (see README.md, 'Setting up Windows')."
}
New-Item -ItemType Directory -Force -Path $OutputDirectory | Out-Null

$commonOptions = @('-hide_banner', '-loglevel', 'error', '-nostdin', '-y')
$durationInSeconds = 20
$backgroundPath = Join-Path $OutputDirectory 'background.png'
# A slow pan over a still Mandelbrot picture larger than 4K, plus grain that
# changes every frame: detail everywhere, like real footage.
$picture = "[0]crop=3840:2160:x='16*t':y='9*t',noise=alls=12:allf=t,format=yuv420p[picture]"
$sound = "sine=frequency=440:beep_factor=4:sample_rate=48000:duration=$durationInSeconds"

Write-Host 'Drawing the background...'
& ffmpeg @commonOptions -f lavfi -i 'mandelbrot=size=4224x2400:start_scale=2.5' -frames:v 1 $backgroundPath
if ($LASTEXITCODE -ne 0) { throw 'ffmpeg failed to draw the background.' }

function New-BenchmarkClip {
    param(
        [Parameter(Mandatory)] [string] $FileName,
        [Parameter(Mandatory)] [string] $Description,
        [Parameter(Mandatory)] [string[]] $VideoOptions
    )
    $outputPath = Join-Path $OutputDirectory $FileName
    Write-Host "Encoding $FileName - $Description (a minute or two)..."
    & ffmpeg @commonOptions -loop 1 -framerate 30 -i $backgroundPath -f lavfi -i $sound `
        -filter_complex $picture -map '[picture]' -map 1:a -t $durationInSeconds -r 30 `
        @VideoOptions -c:a aac -b:a 160k $outputPath
    if ($LASTEXITCODE -ne 0) {
        throw "ffmpeg failed while encoding $FileName (exit code $LASTEXITCODE)."
    }
    Write-Host "Created $outputPath" -ForegroundColor Green
}

New-BenchmarkClip -FileName '4k_h264_60mbps.mp4' -Description 'H.264, 8-bit, 60 Mbit/s' -VideoOptions @(
    '-c:v', 'libx264', '-preset', 'veryfast', '-pix_fmt', 'yuv420p',
    '-b:v', '60M', '-maxrate', '60M', '-bufsize', '120M', '-g', '60'
)

New-BenchmarkClip -FileName '4k_hevc_10bit.mp4' -Description 'HEVC Main 10, 50 Mbit/s' -VideoOptions @(
    '-c:v', 'libx265', '-preset', 'ultrafast', '-pix_fmt', 'yuv420p10le', '-tag:v', 'hvc1',
    '-b:v', '50M', '-x265-params', 'keyint=30:log-level=error'
)

Write-Host "Done. The clips are in $OutputDirectory"
