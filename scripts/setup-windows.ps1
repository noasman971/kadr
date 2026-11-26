<#
.SYNOPSIS
    Checks that a Windows machine has everything needed to build Kadr.

.DESCRIPTION
    Read-only: this script installs nothing and changes nothing. For each tool it
    prints OK, MISSING (needed now) or LATER (needed in a later phase), and the
    command to install what is missing. See README.md, "Setting up Windows".

    Exit code 0 when every tool needed today is present, 1 otherwise.

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File scripts/setup-windows.ps1
#>

$ErrorActionPreference = 'Stop'

# Rust installs its tools in %USERPROFILE%\.cargo\bin. A terminal opened before the
# installation does not have it on its PATH yet, so look there too.
$cargoBinDirectory = Join-Path $env:USERPROFILE '.cargo\bin'
if ((Test-Path $cargoBinDirectory) -and ($env:PATH -notlike "*$cargoBinDirectory*")) {
    $env:PATH = "$cargoBinDirectory;$env:PATH"
}

$script:missingRequiredToolCount = 0

function Write-CheckResult {
    param(
        [Parameter(Mandatory)] [string] $ToolName,
        [Parameter(Mandatory)] [bool] $IsPresent,
        [Parameter(Mandatory)] [string] $Details,
        [Parameter(Mandatory)] [string] $HowToInstall,
        [string] $NeededFromPhase = ''
    )

    if ($IsPresent) {
        Write-Host ("[OK]      {0,-26} {1}" -f $ToolName, $Details) -ForegroundColor Green
        return
    }

    if ($NeededFromPhase -ne '') {
        Write-Host ("[LATER]   {0,-26} needed from {1}. {2}" -f $ToolName, $NeededFromPhase, $Details) -ForegroundColor Yellow
    }
    else {
        Write-Host ("[MISSING] {0,-26} {1}" -f $ToolName, $Details) -ForegroundColor Red
        $script:missingRequiredToolCount++
    }
    Write-Host "          Install: $HowToInstall"
}

function Get-CommandVersionLine {
    param([Parameter(Mandatory)] [string] $CommandName, [string[]] $VersionArguments = @('--version'))

    if (-not (Get-Command $CommandName -ErrorAction SilentlyContinue)) {
        return $null
    }
    $output = & $CommandName @VersionArguments 2>$null | Select-Object -First 1
    return [string] $output
}

function Find-LibclangDirectory {
    $candidateDirectories = @($env:LIBCLANG_PATH, 'C:\Program Files\LLVM\bin') | Where-Object { $_ }
    foreach ($directory in $candidateDirectories) {
        if (Test-Path (Join-Path $directory 'libclang.dll')) {
            return $directory
        }
    }
    return $null
}

function Test-FfmpegDevelopmentFiles {
    if (-not $env:FFMPEG_DIR) {
        return $false
    }
    $headerPath = Join-Path $env:FFMPEG_DIR 'include\libavcodec\avcodec.h'
    $libraryPath = Join-Path $env:FFMPEG_DIR 'lib\avcodec.lib'
    return (Test-Path $headerPath) -and (Test-Path $libraryPath)
}

function Find-MsvcBuildTools {
    $vswherePath = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    if (-not (Test-Path $vswherePath)) {
        return $null
    }
    $installationName = & $vswherePath -products '*' -requires 'Microsoft.VisualStudio.Component.VC.Tools.x86.x64' -property displayName | Select-Object -First 1
    return $installationName
}

Write-Host ''
Write-Host 'Kadr - checking the Windows development environment' -ForegroundColor Cyan
Write-Host ''

# --- Needed now -----------------------------------------------------------------

$msvcInstallationName = Find-MsvcBuildTools
Write-CheckResult -ToolName 'MSVC C++ Build Tools' -IsPresent ([bool] $msvcInstallationName) `
    -Details $(if ($msvcInstallationName) { $msvcInstallationName } else { 'the Rust MSVC toolchain needs the C++ linker' }) `
    -HowToInstall 'winget install Microsoft.VisualStudio.2022.BuildTools --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"'

foreach ($rustTool in @(
        @{ Name = 'rustc'; Arguments = @('--version') },
        @{ Name = 'cargo'; Arguments = @('--version') },
        @{ Name = 'cargo-clippy'; Arguments = @('--version') },
        @{ Name = 'rustfmt'; Arguments = @('--version') })) {
    $versionLine = Get-CommandVersionLine -CommandName $rustTool.Name -VersionArguments $rustTool.Arguments
    Write-CheckResult -ToolName $rustTool.Name -IsPresent ([bool] $versionLine) `
        -Details $(if ($versionLine) { $versionLine } else { 'part of the Rust toolchain' }) `
        -HowToInstall 'winget install Rustlang.Rustup   (then open a new terminal)'
}

$gitVersionLine = Get-CommandVersionLine -CommandName 'git'
Write-CheckResult -ToolName 'git' -IsPresent ([bool] $gitVersionLine) `
    -Details $(if ($gitVersionLine) { $gitVersionLine } else { 'version control' }) `
    -HowToInstall 'winget install Git.Git'

$ffmpegVersionLine = Get-CommandVersionLine -CommandName 'ffmpeg' -VersionArguments @('-version')
Write-CheckResult -ToolName 'ffmpeg (command line)' -IsPresent ([bool] $ffmpegVersionLine) `
    -Details $(if ($ffmpegVersionLine) { $ffmpegVersionLine } else { 'used to generate the test videos' }) `
    -HowToInstall 'winget install Gyan.FFmpeg'

$libclangDirectory = Find-LibclangDirectory
Write-CheckResult -ToolName 'LLVM / libclang' -IsPresent ([bool] $libclangDirectory) `
    -Details $(if ($libclangDirectory) { "libclang.dll found in $libclangDirectory" } else { 'used by bindgen to compile the FFmpeg bindings' }) `
    -HowToInstall 'winget install LLVM.LLVM'

$hasFfmpegDevelopmentFiles = Test-FfmpegDevelopmentFiles
Write-CheckResult -ToolName 'FFmpeg headers + libs' -IsPresent $hasFfmpegDevelopmentFiles `
    -Details $(if ($hasFfmpegDevelopmentFiles) { "FFMPEG_DIR = $env:FFMPEG_DIR" } else { 'FFMPEG_DIR must point to a "shared" FFmpeg 7+ build containing include\ and lib\' }) `
    -HowToInstall 'winget install BtbN.FFmpeg.GPL.Shared.9.0, then set FFMPEG_DIR (see README.md, "FFmpeg development files")'

# --- Needed in later phases -----------------------------------------------------

$nodeVersionLine = Get-CommandVersionLine -CommandName 'node'
Write-CheckResult -ToolName 'Node.js' -IsPresent ([bool] $nodeVersionLine) `
    -Details $(if ($nodeVersionLine) { $nodeVersionLine } else { 'builds the desktop interface.' }) `
    -HowToInstall 'winget install OpenJS.NodeJS.LTS' -NeededFromPhase 'phase 6'

Write-Host ''
if ($script:missingRequiredToolCount -eq 0) {
    Write-Host 'Everything needed right now is installed. Next: cargo build --workspace' -ForegroundColor Green
    exit 0
}

Write-Host "$script:missingRequiredToolCount required tool(s) missing. Install them, open a new terminal and run this script again." -ForegroundColor Red
exit 1
