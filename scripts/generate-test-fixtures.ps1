<#
.SYNOPSIS
    Generates the small test videos used by Kadr's tests.

.DESCRIPTION
    Every fixture is synthesised by FFmpeg from built-in test sources, so nothing
    heavy is committed to Git. The files land in tests/fixtures/generated/
    (ignored by Git). Run this script again whenever the list below changes.

    Works with Windows PowerShell 5.1 and PowerShell 7 (Windows, Linux, macOS).
    Requires the `ffmpeg` command on the PATH, built with libx264.

.EXAMPLE
    ./scripts/generate-test-fixtures.ps1
#>

$ErrorActionPreference = 'Stop'

$repositoryRoot = Split-Path -Parent $PSScriptRoot
$outputDirectory = Join-Path $repositoryRoot 'tests/fixtures/generated'

# Settings shared by every fixture: quiet output, overwrite, no keyboard input,
# and widely compatible H.264 / AAC encoding.
$commonInputOptions = @('-hide_banner', '-loglevel', 'error', '-nostdin', '-y')
$videoEncodingOptions = @('-c:v', 'libx264', '-preset', 'veryfast', '-pix_fmt', 'yuv420p')
$audioEncodingOptions = @('-c:a', 'aac', '-b:a', '128k')

function Test-FfmpegIsAvailable {
    if (-not (Get-Command 'ffmpeg' -ErrorAction SilentlyContinue)) {
        throw "ffmpeg was not found on the PATH. Install it first (see README.md, 'Setting up Windows')."
    }
}

function New-TestFixture {
    param(
        [Parameter(Mandatory)] [string] $FileName,
        [Parameter(Mandatory)] [string] $Description,
        [Parameter(Mandatory)] [string[]] $FfmpegArguments
    )

    $outputPath = Join-Path $outputDirectory $FileName
    Write-Host "Generating $FileName - $Description"

    & ffmpeg @commonInputOptions @FfmpegArguments $outputPath
    if ($LASTEXITCODE -ne 0) {
        throw "ffmpeg failed while generating $FileName (exit code $LASTEXITCODE)."
    }
}

Test-FfmpegIsAvailable
New-Item -ItemType Directory -Force -Path $outputDirectory | Out-Null

New-TestFixture `
    -FileName 'counter_1080p30_beep.mp4' `
    -Description '5 s, 1920x1080, 30 fps, frame counter, 1 kHz beep every second' `
    -FfmpegArguments (@(
        '-f', 'lavfi', '-i', 'testsrc=size=1920x1080:rate=30:duration=5',
        '-f', 'lavfi', '-i', 'sine=frequency=1000:beep_factor=4:duration=5:sample_rate=48000'
    ) + $videoEncodingOptions + $audioEncodingOptions + @('-shortest'))

New-TestFixture `
    -FileName 'colour_bars_720p25_tone.mp4' `
    -Description '3 s, 1280x720, 25 fps, SMPTE colour bars, continuous 440 Hz tone' `
    -FfmpegArguments (@(
        '-f', 'lavfi', '-i', 'smptehdbars=size=1280x720:rate=25:duration=3',
        '-f', 'lavfi', '-i', 'sine=frequency=440:duration=3:sample_rate=48000'
    ) + $videoEncodingOptions + $audioEncodingOptions + @('-shortest'))

New-TestFixture `
    -FileName 'vertical_1080x1920_30_no_audio.mp4' `
    -Description '3 s, 1080x1920 (9:16), 30 fps, no audio track' `
    -FfmpegArguments (@(
        '-f', 'lavfi', '-i', 'testsrc2=size=1080x1920:rate=30:duration=3'
    ) + $videoEncodingOptions + @('-an'))

# A textured square moves left to right on a grey background, bobbing up and down,
# and passes behind a black bar around t = 4 s. Used to test the region tracker,
# including the "target lost behind an obstacle" case.
New-TestFixture `
    -FileName 'moving_target_720p30.mp4' `
    -Description '6 s, 1280x720, 30 fps, moving textured square hidden by an obstacle near t = 4 s' `
    -FfmpegArguments (@(
        '-f', 'lavfi', '-i', 'color=color=0x404040:size=1280x720:rate=30:duration=6',
        '-f', 'lavfi', '-i', 'testsrc2=size=120x120:rate=30:duration=6',
        '-filter_complex', "[0:v][1:v]overlay=x='100+150*t':y='300+80*sin(2*t)':shortest=1,drawbox=x=700:y=200:w=120:h=320:color=black:t=fill[video]",
        '-map', '[video]'
    ) + $videoEncodingOptions + @('-an'))

# Tone from 0 to 2 s, silence from 2 to 4 s, tone again from 4 to 6 s.
# Used to test silence detection and removal.
New-TestFixture `
    -FileName 'tone_silence_tone_360p30.mp4' `
    -Description '6 s, 640x360, 30 fps, 440 Hz tone with a silent gap from 2 s to 4 s' `
    -FfmpegArguments (@(
        '-f', 'lavfi', '-i', 'testsrc=size=640x360:rate=30:duration=6',
        '-f', 'lavfi', '-i', "aevalsrc='if(between(t,2,4),0,0.5*sin(2*PI*440*t))':sample_rate=48000:duration=6"
    ) + $videoEncodingOptions + $audioEncodingOptions + @('-shortest'))

Write-Host "Done. Fixtures are in $outputDirectory"
