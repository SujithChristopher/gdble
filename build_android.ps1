# Build GdBLE for Android arm64-v8a using Podman
$ErrorActionPreference = "Stop"

# Use location of script as working directory
$scriptPath = Split-Path -Parent $MyInvocation.MyCommand.Definition
Set-Location $scriptPath

# Check if Podman is available, and try to start the machine if not
try {
    # Check connectability to podman
    & podman info > $null 2>&1
    if ($LASTEXITCODE -ne 0) {
        Write-Host "Podman machine not running. Attempting to start default machine..." -ForegroundColor Yellow
        & podman machine start
        
        # Wait up to 30 seconds for the service to actually become available
        Write-Host "Waiting for Podman service to initialize..." -ForegroundColor Cyan
        $retry_count = 0
        while ($retry_count -lt 6) {
            Start-Sleep -Seconds 5
            & podman info > $null 2>&1
            if ($LASTEXITCODE -eq 0) { break }
            $retry_count++
        }

        if ($LASTEXITCODE -ne 0) {
            Write-Host "Failed to start Podman machine or service timed out. Please run 'podman machine init' or check 'podman machine list'." -ForegroundColor Red
            exit 1
        }
    }
} catch {
    Write-Host "Podman command not found. Please ensure Podman is installed and in your PATH." -ForegroundColor Red
    exit 1
}

# Build the build container
Write-Host "Building Podman image..." -ForegroundColor Cyan
podman build -t gdble-android-builder -f Containerfile .

# Build the extension for Android
# On Windows, we need to ensure the volume path is handled correctly by Podman
$currentDir = (Get-Item .).FullName
# Replace backslashes with forward slashes for container compatibility if needed, 
# but Podman on Windows usually handles absolute Windows paths.
Write-Host "Building GdBLE for Android (aarch64-linux-android)..." -ForegroundColor Cyan
podman run --rm -v "${currentDir}:/workspace" gdble-android-builder `
    cargo build --release --target aarch64-linux-android

# Create destination directories
$targetBinSubdir = "android"
$localBin = "addons/gdble/bin/$targetBinSubdir"
$projectBin = "../addons/gdble/bin/$targetBinSubdir"

if (!(Test-Path $localBin)) { New-Item -ItemType Directory -Path $localBin -Force }
if (!(Test-Path $projectBin)) { New-Item -ItemType Directory -Path $projectBin -Force }

# Copy the resulting library
$src = "target/aarch64-linux-android/release/libgdble.so"
if (Test-Path $src) {
    Copy-Item $src "$localBin/libgdble.so" -Force
    Copy-Item $src "$projectBin/libgdble.so" -Force
    Write-Host "Successfully built and deployed Android library to $localBin" -ForegroundColor Green
} else {
    Write-Host "Error: Library not found at $src" -ForegroundColor Red
    exit 1
}
