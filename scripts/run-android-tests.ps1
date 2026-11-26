<#
.SYNOPSIS
    Builds Kadr's tests for Android, copies them to a phone or emulator, runs them
    there and reports the result.

.DESCRIPTION
    Rust test programs are ordinary command-line programs: built for Android with
    cargo-ndk, they run in `adb shell` like on a desktop, with the phone's real
    codecs (MediaCodec) and GPU drivers (Vulkan or OpenGL ES).

    1. Builds the test programs of kadr-core, kadr-vision, kadr-render and kadr-media
       (MediaCodec backend, without FFmpeg) for the chosen ABI.
    2. If FFmpeg is set up on this machine, writes a video with the desktop backend
       (B-frames, AAC encoder delay) so the phone checks it reads such files exactly.
    3. Copies everything to /data/local/tmp/kadr-tests on the device, runs each test
       program, then deletes the folder.

    Needs: one device in `adb devices` (a phone with USB debugging, or a running
    emulator), ANDROID_NDK_HOME (or ANDROID_HOME with an NDK), cargo-ndk, and the
    Rust target of the ABI. See docs/android.md.

.PARAMETER Abi
    `x86_64` for the emulator on a PC (default), `arm64-v8a` for a phone.

.EXAMPLE
    ./scripts/run-android-tests.ps1

.EXAMPLE
    ./scripts/run-android-tests.ps1 -Abi arm64-v8a
#>

param(
    [ValidateSet('x86_64', 'arm64-v8a')]
    [string] $Abi = 'x86_64'
)

$ErrorActionPreference = 'Stop'

# Oldest supported Android: API 28, Android 9 (decision record 0041).
$minimumApiLevel = 28
$deviceDirectory = '/data/local/tmp/kadr-tests'
$repositoryRoot = Split-Path -Parent $PSScriptRoot

function Find-Adb {
    $adb = Get-Command 'adb' -ErrorAction SilentlyContinue
    if ($adb) { return $adb.Source }
    $sdkRoot = if ($env:ANDROID_HOME) { $env:ANDROID_HOME } else { Join-Path $env:LOCALAPPDATA 'Android\Sdk' }
    $candidate = Join-Path $sdkRoot 'platform-tools/adb.exe'
    if (-not (Test-Path $candidate)) { $candidate = Join-Path $sdkRoot 'platform-tools/adb' }
    if (Test-Path $candidate) { return $candidate }
    throw 'adb was not found: install the Android SDK platform tools (see docs/android.md).'
}

function Assert-OneDevice([string] $adb) {
    $devices = & $adb devices | Select-Object -Skip 1 | Where-Object { $_ -match '\tdevice$' }
    if (@($devices).Count -ne 1) {
        throw "Exactly one Android device must be connected (found $(@($devices).Count)). Start the emulator or plug in a phone with USB debugging."
    }
}

function Build-TestPrograms {
    Write-Host "Building the test programs for $Abi (API $minimumApiLevel)..."
    $arguments = @(
        'ndk', '--target', $Abi, '--platform', $minimumApiLevel,
        'test', '--no-run', '--message-format=json',
        '-p', 'kadr-core', '-p', 'kadr-vision', '-p', 'kadr-render', '-p', 'kadr-media',
        '--no-default-features', '--features', 'kadr-media/mediacodec'
    )
    $messages = & cargo @arguments
    if ($LASTEXITCODE -ne 0) { throw 'Building the test programs failed (see above).' }
    $programs = foreach ($line in $messages) {
        if (-not $line.StartsWith('{')) { continue }
        $message = $line | ConvertFrom-Json
        if ($message.reason -eq 'compiler-artifact' -and $message.profile.test -and $message.executable) {
            $message.executable
        }
    }
    return @($programs | Sort-Object -Unique)
}

function New-DesktopMadeVideo([string] $path) {
    Write-Host 'Writing a video with the desktop FFmpeg backend...'
    & cargo run --quiet -p kadr-media --example write_test_video -- $path
    if ($LASTEXITCODE -ne 0) {
        Write-Warning 'FFmpeg is not set up on this machine: the desktop-made video test is skipped.'
        return $false
    }
    return $true
}

Push-Location $repositoryRoot
try {
    $adb = Find-Adb
    Assert-OneDevice $adb
    $programs = Build-TestPrograms
    if ($programs.Count -eq 0) { throw 'No test program was built.' }

    $temporaryDirectory = Join-Path ([System.IO.Path]::GetTempPath()) "kadr-android-tests-$PID"
    New-Item -ItemType Directory -Force $temporaryDirectory | Out-Null
    $desktopVideo = Join-Path $temporaryDirectory 'desktop_made.mp4'
    $hasDesktopVideo = New-DesktopMadeVideo $desktopVideo

    & $adb shell "rm -rf $deviceDirectory && mkdir -p $deviceDirectory" | Out-Null
    foreach ($program in $programs) {
        & $adb push $program "$deviceDirectory/" | Out-Null
    }
    $environment = ''
    if ($hasDesktopVideo) {
        & $adb push $desktopVideo "$deviceDirectory/desktop_made.mp4" | Out-Null
        $environment = "KADR_DESKTOP_MADE_VIDEO=$deviceDirectory/desktop_made.mp4 "
    }

    $failedPrograms = @()
    foreach ($program in $programs) {
        $name = Split-Path -Leaf $program
        Write-Host "`n=== $name"
        # Tests share the device: one at a time keeps codec and GPU use predictable.
        & $adb shell "cd $deviceDirectory && chmod +x ./$name && ${environment}TMPDIR=$deviceDirectory ./$name --test-threads=1; echo KADR_EXIT_CODE=`$?" |
            Tee-Object -Variable output | Write-Host
        if (-not ($output -match 'KADR_EXIT_CODE=0')) { $failedPrograms += $name }
    }
    & $adb shell "rm -rf $deviceDirectory" | Out-Null
    Remove-Item -Recurse -Force $temporaryDirectory

    if ($failedPrograms.Count -gt 0) {
        throw "Tests failed on the device in: $($failedPrograms -join ', ')"
    }
    Write-Host "`nAll $($programs.Count) test programs passed on the device."
}
finally {
    Pop-Location
}
