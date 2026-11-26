<#
.SYNOPSIS
    Generates the 1080p video used to measure the region tracker on a real
    (encoded) video whose true trajectory is known.

.DESCRIPTION
    A 200 x 240 test pattern (FFmpeg's testsrc2) moves over a detailed, still
    background (a Mandelbrot picture) for 60 seconds at 30 frames per second:
        left = 860 + 500 sin(2 pi t / 10)
        top  = 420 + 200 sin(2 pi t / 7)
    The file lands in tests/fixtures/generated/tracking/ (ignored by Git).

    Measure the tracker on it:
        cargo run --release -p kadr-engine --example track_video -- `
            tests/fixtures/generated/tracking/moving_box_1080p.mp4 2.5 1360,576,200,240 > track.csv
        python scripts/compare-tracking-with-truth.py track.csv

    Requires the `ffmpeg` command on the PATH, built with libx264.

.EXAMPLE
    ./scripts/generate-tracking-benchmark.ps1
#>

$ErrorActionPreference = 'Stop'

$repositoryRoot = Split-Path -Parent $PSScriptRoot
$outputDirectory = Join-Path $repositoryRoot 'tests/fixtures/generated/tracking'
New-Item -ItemType Directory -Force -Path $outputDirectory | Out-Null

if (-not (Get-Command 'ffmpeg' -ErrorAction SilentlyContinue)) {
    throw "ffmpeg was not found on the PATH. Install it first (see README.md, 'Setting up Windows')."
}

$backgroundPath = Join-Path $outputDirectory 'background.png'
$videoPath = Join-Path $outputDirectory 'moving_box_1080p.mp4'
$commonOptions = @('-hide_banner', '-loglevel', 'error', '-nostdin', '-y')

Write-Host 'Drawing the background...'
& ffmpeg @commonOptions -f lavfi -i 'mandelbrot=s=1920x1080:start_scale=2.5' -frames:v 1 $backgroundPath
if ($LASTEXITCODE -ne 0) { throw 'ffmpeg failed to draw the background.' }

Write-Host 'Encoding the 60-second video (about a minute)...'
$overlay = "[0][1]overlay=x='860+500*sin(2*PI*t/10)':y='420+200*sin(2*PI*t/7)':shortest=1,format=yuv420p"
& ffmpeg @commonOptions -loop 1 -framerate 30 -i $backgroundPath -f lavfi -i 'testsrc2=s=200x240:r=30' `
    -filter_complex $overlay -t 60 -r 30 -c:v libx264 -preset veryfast -crf 20 $videoPath
if ($LASTEXITCODE -ne 0) { throw 'ffmpeg failed to encode the video.' }

Write-Host "Created $videoPath" -ForegroundColor Green
Write-Host 'Selection at 2.5 s: 1360,576,200,240 (left, top, width, height).'
