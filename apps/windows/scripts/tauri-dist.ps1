param(
    [ValidateSet("x64","arm64")][string]$Architecture = "x64",
    [ValidateSet("cpu","vulkan","cuda")][string]$Backend = "cpu"
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    & npm.cmd run build:web
    if ($LASTEXITCODE) { throw 'Web build failed' }
    & "$PSScriptRoot/build-native.ps1" -Architecture $Architecture -Backend $Backend
    if ($LASTEXITCODE) { throw 'Native build failed' }
    $target = if ($Architecture -eq 'arm64') { 'aarch64-pc-windows-msvc' } else { 'x86_64-pc-windows-msvc' }
    & npx.cmd --no-install tauri build --target $target --bundles nsis -- --locked
    if ($LASTEXITCODE) { throw 'Tauri packaging failed' }
    $targetDir = (& cargo metadata --no-deps --format-version 1 --manifest-path src-tauri/Cargo.toml | ConvertFrom-Json).target_directory
    $installer = @(Get-ChildItem "$targetDir/$target/release/bundle/nsis" -File -Filter '*-setup.exe')
    if ($installer.Count -ne 1) { throw 'Expected exactly one Tauri NSIS installer' }
    $out = Join-Path $root "release/$Architecture-$Backend"
    New-Item -Type Directory -Force $out | Out-Null
    Copy-Item $installer[0].FullName $out -Force
} finally {
    Pop-Location
}
