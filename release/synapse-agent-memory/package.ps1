param(
    [string]$Bin = "",
    [string]$Target = "x86_64-pc-windows-msvc",
    [string]$OutDir = "",
    [switch]$AllowDirty
)

$ErrorActionPreference = "Stop"
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$RepoRoot = (Resolve-Path (Join-Path $ScriptDir "..\..")).Path
if (-not $OutDir) { $OutDir = Join-Path $RepoRoot "release\dist-agent-memory" }
if (-not $Target.EndsWith("-pc-windows-msvc")) { throw "package.ps1 only packages Windows MSVC targets" }

$Dirty = [bool](git -C $RepoRoot status --porcelain)
if ($Dirty -and -not $AllowDirty) {
    throw "Refusing to package a dirty worktree; commit intended changes or pass -AllowDirty for a local smoke"
}

if (-not $Bin) {
    cargo build --manifest-path (Join-Path $RepoRoot "Cargo.toml") --locked --profile release-hardened --target $Target -p synapse-cli --no-default-features
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
    $Bin = Join-Path $RepoRoot "target\$Target\release-hardened\synx.exe"
}
if (-not (Test-Path -LiteralPath $Bin -PathType Leaf)) { throw "Rust binary not found: $Bin" }
$Bytes = [System.IO.File]::ReadAllBytes($Bin)
if ($Bytes.Length -lt 2 -or $Bytes[0] -ne 0x4d -or $Bytes[1] -ne 0x5a) {
    throw "Refusing non-native Windows executable: $Bin"
}

$Version = ((& $Bin --version) -split "\s+")[-1]
if ($LASTEXITCODE -ne 0 -or -not $Version) { throw "Binary did not report a version" }
$Commit = (git -C $RepoRoot rev-parse --short=12 HEAD).Trim()
$Temp = Join-Path ([System.IO.Path]::GetTempPath()) ("synapse-agent-memory-package-" + [guid]::NewGuid())
$Root = "synapse-agent-memory-$Target"
$Stage = Join-Path $Temp $Root
New-Item -ItemType Directory -Force -Path $Stage | Out-Null
try {
    Copy-Item -LiteralPath $Bin -Destination (Join-Path $Stage "synx.exe")
    Copy-Item -LiteralPath (Join-Path $ScriptDir "README.md") -Destination (Join-Path $Stage "README.md")
    Copy-Item -LiteralPath (Join-Path $ScriptDir "THIRD-PARTY-LICENSES.html") -Destination (Join-Path $Stage "THIRD-PARTY-LICENSES.html")
    Copy-Item -LiteralPath (Join-Path $RepoRoot "NOTICE") -Destination (Join-Path $Stage "NOTICE")
    Copy-Item -LiteralPath (Join-Path $RepoRoot "ATTRIBUTIONS.md") -Destination (Join-Path $Stage "ATTRIBUTIONS.md")
    $LicenseDir = Join-Path $Stage "LICENSES"
    New-Item -ItemType Directory -Force -Path $LicenseDir | Out-Null
    Copy-Item -LiteralPath (Join-Path $RepoRoot "LICENSES\MIT.txt") -Destination (Join-Path $LicenseDir "MIT.txt")
    Copy-Item -LiteralPath (Join-Path $RepoRoot "LICENSES\FSL-1.1-ALv2.txt") -Destination (Join-Path $LicenseDir "FSL-1.1-ALv2.txt")
    [ordered]@{
        product = "synapse-agent-memory"
        binary = "synx.exe"
        version = $Version
        target = $Target
        profile = "portable"
        semantic_embeddings = $false
        network = $false
        age_encryption = $false
        pdf_ingest = $false
        sharding = $false
        proprietary_engine = $false
        first_party_licenses = @("FSL-1.1-ALv2", "MIT")
        git_commit = $Commit
        dirty = $Dirty
    } | ConvertTo-Json | Set-Content -Encoding utf8 (Join-Path $Stage "BUILD-INFO.json")

    New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
    $Asset = "synapse-agent-memory-$Target.zip"
    $Archive = Join-Path $OutDir $Asset
    Compress-Archive -LiteralPath $Stage -DestinationPath $Archive -Force
    $Hash = (Get-FileHash -Algorithm SHA256 $Archive).Hash.ToLowerInvariant()
    "$Hash  $Asset" | Set-Content -Encoding ascii "$Archive.sha256"
    [pscustomobject]@{ archive = $Archive; checksum = "$Archive.sha256" } | ConvertTo-Json
} finally {
    Remove-Item -LiteralPath $Temp -Recurse -Force -ErrorAction SilentlyContinue
}
