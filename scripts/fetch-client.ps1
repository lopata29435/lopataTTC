# Downloads the matching trusttunnel_client binary from the official GitHub
# release into ./src-tauri/resources/ so `cargo tauri build` can bundle it.
#
# Usage:
#   .\scripts\fetch-client.ps1 -AssetName "trusttunnel_client-v1.0.49-windows-x86_64.zip"
#   .\scripts\fetch-client.ps1 -Os windows -Arch x86_64
#
# If only Os/Arch given, the script queries the latest release tag from GitHub
# and constructs the filename. Wintun is downloaded too for Windows targets.

[CmdletBinding(DefaultParameterSetName = 'ByAsset')]
param(
    [Parameter(ParameterSetName='ByAsset', Position=0)]
    [string]$AssetName,

    [Parameter(ParameterSetName='ByOsArch')]
    [ValidateSet('windows','linux','macos')]
    [string]$Os,

    [Parameter(ParameterSetName='ByOsArch')]
    [ValidateSet('x86_64','i686','aarch64','armv7','universal')]
    [string]$Arch,

    [string]$Tag
)

$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
$resourcesDir = Join-Path $root 'src-tauri/resources'
New-Item -ItemType Directory -Force -Path $resourcesDir | Out-Null

if (-not $Tag) {
    Write-Host "Fetching latest release tag..."
    $rel = Invoke-RestMethod -Uri 'https://api.github.com/repos/TrustTunnel/TrustTunnelClient/releases/latest' -Headers @{
        'User-Agent' = 'trusttunnel-gui-ci'
        'Accept'     = 'application/vnd.github+json'
    }
    $Tag = $rel.tag_name
    Write-Host "Latest tag: $Tag"
}

if (-not $AssetName) {
    if (-not $Os -or -not $Arch) {
        throw "Provide either -AssetName or both -Os and -Arch"
    }
    $ext = if ($Os -eq 'windows') { 'zip' } else { 'tar.gz' }
    if ($Os -eq 'macos') {
        $AssetName = "trusttunnel_client-$Tag-macos-universal.tar.gz"
    } else {
        $AssetName = "trusttunnel_client-$Tag-$Os-$Arch.$ext"
    }
}

$url = "https://github.com/TrustTunnel/TrustTunnelClient/releases/download/$Tag/$AssetName"
$dest = Join-Path $resourcesDir $AssetName
Write-Host "Downloading: $url"
Invoke-WebRequest -Uri $url -OutFile $dest -UseBasicParsing

Write-Host "Extracting $AssetName..."
if ($AssetName.EndsWith('.zip')) {
    Expand-Archive -Path $dest -DestinationPath $resourcesDir -Force
} elseif ($AssetName.EndsWith('.tar.gz')) {
    tar -xzf $dest -C $resourcesDir
}
Remove-Item $dest -Force

# Flatten: if archive produced a subdir, lift trusttunnel_client(.exe) to resources/
$binName = if ($AssetName -like '*windows*') { 'trusttunnel_client.exe' } else { 'trusttunnel_client' }
$found = Get-ChildItem -Path $resourcesDir -Recurse -Filter $binName | Select-Object -First 1
if ($found -and $found.DirectoryName -ne $resourcesDir) {
    Move-Item -Path $found.FullName -Destination (Join-Path $resourcesDir $binName) -Force
}

# Wintun for Windows targets — pull from the asset if not already extracted.
if ($AssetName -like '*windows*') {
    $wintun = Get-ChildItem -Path $resourcesDir -Recurse -Filter 'wintun.dll' | Select-Object -First 1
    if ($wintun -and $wintun.DirectoryName -ne $resourcesDir) {
        Move-Item -Path $wintun.FullName -Destination (Join-Path $resourcesDir 'wintun.dll') -Force
    }
}

# Remove any leftover extracted subdirectories (we already lifted the files we need).
Get-ChildItem -Path $resourcesDir -Directory -ErrorAction SilentlyContinue |
    Where-Object { $_.Name -like 'trusttunnel_client-*' } |
    Remove-Item -Recurse -Force

Write-Host "Done. resources/:"
Get-ChildItem $resourcesDir | Select-Object Name, Length | Format-Table
