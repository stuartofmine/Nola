param(
    [ValidateSet("x64", "arm64")]
    [string]$Architecture = "x64",

    [ValidateSet("cpu", "vulkan", "cuda")]
    [string]$Backend = "cpu"
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

if ($Architecture -eq "arm64" -and $Backend -ne "cpu") {
    throw "The packaged ARM64 baseline supports the CPU backend. Build a custom llama.cpp runtime for other ARM64 backends."
}

$WindowsRoot = Split-Path -Parent $PSScriptRoot
$RepoRoot = Split-Path -Parent $WindowsRoot
$NativeLock = Get-Content (Join-Path $WindowsRoot "native-lock.json") -Raw | ConvertFrom-Json
$LlamaRepository = [string]$NativeLock.llamaCpp.repository
$LlamaVersion = [string]$NativeLock.llamaCpp.release
$LlamaCommitPrefix = [string]$NativeLock.llamaCpp.commitPrefix
$RustTarget = if ($Architecture -eq "arm64") { "aarch64-pc-windows-msvc" } else { "x86_64-pc-windows-msvc" }
$CMakeArchitecture = if ($Architecture -eq "arm64") { "ARM64" } else { "x64" }

function Invoke-Checked {
    param(
        [Parameter(Mandatory = $true)][string]$Command,
        [Parameter(ValueFromRemainingArguments = $true)][string[]]$Arguments
    )
    & $Command @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$Command failed with exit code $LASTEXITCODE"
    }
}

Write-Host "Building Nola Rust runtime for $RustTarget..."
Invoke-Checked rustup target add $RustTarget
Invoke-Checked cargo build --manifest-path (Join-Path $RepoRoot "cli\Cargo.toml") --release --target $RustTarget --bins
$CargoMetadata = (& cargo metadata --manifest-path (Join-Path $RepoRoot "Cargo.toml") --no-deps --format-version 1 | ConvertFrom-Json)
if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace([string]$CargoMetadata.target_directory)) {
    throw "Could not resolve the Cargo workspace target directory"
}
$CargoTargetDirectory = [string]$CargoMetadata.target_directory

$ConfiguredLlamaSource = $env:NOLA_LLAMA_CPP_SOURCE
$LlamaSource = if ([string]::IsNullOrWhiteSpace($ConfiguredLlamaSource)) {
    Join-Path $WindowsRoot ".cache\llama.cpp-$LlamaVersion"
} else {
    [System.IO.Path]::GetFullPath($ConfiguredLlamaSource)
}

if (-not (Test-Path (Join-Path $LlamaSource "CMakeLists.txt"))) {
    if (-not [string]::IsNullOrWhiteSpace($ConfiguredLlamaSource)) {
        throw "NOLA_LLAMA_CPP_SOURCE does not contain a llama.cpp checkout: $LlamaSource"
    }
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $LlamaSource) | Out-Null
    Write-Host "Fetching the pinned llama.cpp release $LlamaVersion for the build cache..."
    Invoke-Checked git clone --branch $LlamaVersion --depth 1 $LlamaRepository $LlamaSource
}

$ResolvedLlamaCommit = (& git -C $LlamaSource rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0) {
    throw "Could not resolve the llama.cpp revision in $LlamaSource"
}
if (-not $ResolvedLlamaCommit.StartsWith($LlamaCommitPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "llama.cpp revision mismatch: expected $LlamaCommitPrefix, found $ResolvedLlamaCommit"
}
Write-Host "Verified llama.cpp $LlamaVersion at $ResolvedLlamaCommit"

$LlamaBuild = Join-Path $WindowsRoot ".cache\llama-build-$LlamaVersion-$Architecture-$Backend"
$ConfigureArguments = @(
    "-S", $LlamaSource,
    "-B", $LlamaBuild,
    "-A", $CMakeArchitecture,
    "-DGGML_NATIVE=OFF",
    "-DLLAMA_CURL=OFF",
    "-DBUILD_SHARED_LIBS=ON",
    "-DCMAKE_BUILD_TYPE=Release"
)

switch ($Backend) {
    "vulkan" { $ConfigureArguments += "-DGGML_VULKAN=ON" }
    "cuda" { $ConfigureArguments += "-DGGML_CUDA=ON" }
}

Write-Host "Building llama.cpp $LlamaVersion ($Architecture/$Backend)..."
Invoke-Checked cmake @ConfigureArguments
Invoke-Checked cmake --build $LlamaBuild --config Release --target llama-server --parallel

$LlamaServer = Get-ChildItem -Path $LlamaBuild -Filter "llama-server.exe" -File -Recurse |
    Sort-Object LastWriteTimeUtc -Descending |
    Select-Object -First 1
if ($null -eq $LlamaServer) {
    throw "llama-server.exe was not produced under $LlamaBuild"
}

$env:NOLA_WINDOWS_CLI_PATH = Join-Path $CargoTargetDirectory "$RustTarget\release\nola.exe"
$env:NOLA_WINDOWS_AGENT_HOST_PATH = Join-Path $CargoTargetDirectory "$RustTarget\release\nola-agent-host.exe"
$env:NOLA_LLAMA_SERVER = $LlamaServer.FullName
$env:NOLA_LLAMA_CPP_SOURCE = $LlamaSource
$env:NOLA_LLAMA_BACKEND = $Backend
$env:NOLA_LLAMA_COMMIT = $ResolvedLlamaCommit

Invoke-Checked node (Join-Path $PSScriptRoot "stage-native.mjs") --arch $Architecture
Write-Host "Native runtime staged for Electron packaging."
