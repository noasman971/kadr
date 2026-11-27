<#
.SYNOPSIS
    Generates the 1080p video used to measure the stabilisation on a real
    (encoded) video whose true camera motion is known.

.DESCRIPTION
    A camera films a detailed still picture (a 2560 x 1440 Mandelbrot picture):
    it rolls, then looks through a 1920 x 1080 window that pans slowly and
    shakes. At 30 frames per second, for t in seconds from the start of a shot:

        roll (degrees, clockwise) = 0.5 sin(2 pi 1.3 t) + 0.25 sin(2 pi 3.7 t + 1)
        window left = 320 + 200 sin(2 pi t / 20)
                      + 10 sin(2 pi 2.3 t) + 6 sin(2 pi 5.1 t + 1) + 3 sin(2 pi 9.7 t + 2)
        window top  = 180 + 8 sin(2 pi 1.7 t + 0.5) + 5 sin(2 pi 4.3 t) + 2 sin(2 pi 8.9 t + 1)

    Two things move in front of the picture, to be ignored by the measurement:
    a small test pattern wandering all the time, and a large one crossing the
    frame from 20 s to 24 s. Noise is added, as a camera would.

    Shot A lasts 40 seconds; shot B, 10 seconds on another picture, follows it:
    there is a hard cut at 40 s. The file lands in
    tests/fixtures/generated/stabilization/ (ignored by Git).

    Measure the stabilisation on it:
        cargo run --release -p kadr-engine --example stabilize_video -- `
            tests/fixtures/generated/stabilization/shaky_camera_1080p.mp4 > path.csv
        python scripts/compare-stabilization-with-truth.py path.csv

    Requires the `ffmpeg` command on the PATH, built with libx264.

.EXAMPLE
    ./scripts/generate-stabilization-benchmark.ps1
#>

$ErrorActionPreference = 'Stop'

$repositoryRoot = Split-Path -Parent $PSScriptRoot
$outputDirectory = Join-Path $repositoryRoot 'tests/fixtures/generated/stabilization'
New-Item -ItemType Directory -Force -Path $outputDirectory | Out-Null

if (-not (Get-Command 'ffmpeg' -ErrorAction SilentlyContinue)) {
    throw "ffmpeg was not found on the PATH. Install it first (see README.md, 'Setting up Windows')."
}

$commonOptions = @('-hide_banner', '-loglevel', 'error', '-nostdin', '-y')
$encoding = @('-r', '30', '-c:v', 'libx264', '-preset', 'veryfast', '-crf', '18')

# The camera: a roll about the picture's centre, then a moving window. These
# formulas are repeated in scripts/compare-stabilization-with-truth.py.
$roll = "rotate=a='(0.5*sin(2*PI*1.3*t)+0.25*sin(2*PI*3.7*t+1))*PI/180':ow=iw:oh=ih:bilinear=1"
$windowLeft = '320+200*sin(2*PI*t/20)+10*sin(2*PI*2.3*t)+6*sin(2*PI*5.1*t+1)+3*sin(2*PI*9.7*t+2)'
$windowTop = '180+8*sin(2*PI*1.7*t+0.5)+5*sin(2*PI*4.3*t)+2*sin(2*PI*8.9*t+1)'
$camera = "$roll,crop=1920:1080:x='$windowLeft':y='$windowTop':exact=1,noise=alls=6:allf=t,format=yuv420p"

$stillA = Join-Path $outputDirectory 'still_a.png'
$stillB = Join-Path $outputDirectory 'still_b.png'
$shotA = Join-Path $outputDirectory 'shot_a.mp4'
$shotB = Join-Path $outputDirectory 'shot_b.mp4'
$shotList = Join-Path $outputDirectory 'shots.txt'
$videoPath = Join-Path $outputDirectory 'shaky_camera_1080p.mp4'

Write-Host 'Drawing the two pictures...'
& ffmpeg @commonOptions -f lavfi -i 'mandelbrot=s=2560x1440:start_scale=2.5' -frames:v 1 $stillA
if ($LASTEXITCODE -ne 0) { throw 'ffmpeg failed to draw the first picture.' }
& ffmpeg @commonOptions -f lavfi -i 'mandelbrot=s=2560x1440:start_scale=2.5:start_x=-0.2:start_y=0.65' -frames:v 1 $stillB
if ($LASTEXITCODE -ne 0) { throw 'ffmpeg failed to draw the second picture.' }

Write-Host 'Encoding shot A, 40 seconds (a few minutes)...'
$sceneA = "[0][1]overlay=x='1100+600*sin(2*PI*t/9)':y='600+250*sin(2*PI*t/6)'[small];" +
    "[small][2]overlay=x='200+(t-20)*550':y=360:enable='between(t,20,24)'[scene];" +
    "[scene]$camera"
& ffmpeg @commonOptions -loop 1 -framerate 30 -i $stillA `
    -f lavfi -i 'testsrc2=s=320x240:r=30' -f lavfi -i 'testsrc2=s=640x720:r=30' `
    -filter_complex $sceneA -t 40 @encoding $shotA
if ($LASTEXITCODE -ne 0) { throw 'ffmpeg failed to encode shot A.' }

Write-Host 'Encoding shot B, 10 seconds...'
& ffmpeg @commonOptions -loop 1 -framerate 30 -i $stillB -vf $camera -t 10 @encoding $shotB
if ($LASTEXITCODE -ne 0) { throw 'ffmpeg failed to encode shot B.' }

Write-Host 'Joining the two shots...'
Set-Content -Path $shotList -Encoding ascii -Value @("file 'shot_a.mp4'", "file 'shot_b.mp4'")
& ffmpeg @commonOptions -f concat -safe 0 -i $shotList -c copy $videoPath
if ($LASTEXITCODE -ne 0) { throw 'ffmpeg failed to join the shots.' }
Remove-Item $shotA, $shotB, $shotList

Write-Host "Created $videoPath" -ForegroundColor Green
Write-Host 'The cut is at 40 s (frame 1200).'
