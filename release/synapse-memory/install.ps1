param(
    [string]$Version = "",
    [string]$Prefix = "$env:LOCALAPPDATA\Synapse",
    [string]$Database = "$env:USERPROFILE\.synapse\brain.db",
    [switch]$DryRun,
    [switch]$NoPathUpdate
)

$ErrorActionPreference = "Stop"
$DefaultVersion = "1.0.1-rc.1"
$Repo = if ($env:SYNAPSE_REPO) { $env:SYNAPSE_REPO } else { "https://github.com/Supersynergy/synapse" }
$Arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
$Target = switch ($Arch) {
    "X64" { "x86_64-pc-windows-msvc" }
    "Arm64" { "aarch64-pc-windows-msvc" }
    default { throw "Unsupported Windows architecture: $Arch" }
}
$Asset = "synapse-memory-$Target.zip"

if ($env:SYNAPSE_RELEASE_BASE) {
    $Base = $env:SYNAPSE_RELEASE_BASE.TrimEnd("/")
} elseif ($Version) {
    $CleanVersion = $Version -replace "^ctxos-v", "" -replace "^v", ""
    $Base = "$Repo/releases/download/ctxos-v$CleanVersion"
} else {
    $Base = "$Repo/releases/download/ctxos-v$DefaultVersion"
}

if ($DryRun) {
    [pscustomobject]@{
        target = $Target
        asset = $Asset
        archive = "$Base/$Asset"
        checksum = "$Base/$Asset.sha256"
        prefix = $Prefix
        database = $Database
    } | ConvertTo-Json
    exit 0
}

$Temp = Join-Path ([System.IO.Path]::GetTempPath()) ("synapse-memory-" + [guid]::NewGuid())
New-Item -ItemType Directory -Force -Path $Temp | Out-Null
try {
    $Archive = Join-Path $Temp $Asset
    $Checksum = "$Archive.sha256"
    Invoke-WebRequest -UseBasicParsing "$Base/$Asset" -OutFile $Archive
    Invoke-WebRequest -UseBasicParsing "$Base/$Asset.sha256" -OutFile $Checksum

    $Expected = ((Get-Content -Raw $Checksum).Trim() -split "\s+")[0].ToLowerInvariant()
    $Actual = (Get-FileHash -Algorithm SHA256 $Archive).Hash.ToLowerInvariant()
    if (-not $Expected -or $Expected -ne $Actual) {
        throw "Checksum mismatch for $Asset"
    }

    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $Root = "synapse-memory-$Target/"
    $RequiredFiles = @(
        "${Root}synx.exe",
        "${Root}README.md",
        "${Root}THIRD-PARTY-LICENSES.html",
        "${Root}NOTICE",
        "${Root}ATTRIBUTIONS.md",
        "${Root}LICENSES/FSL-1.1-ALv2.txt",
        "${Root}LICENSES/MIT.txt",
        "${Root}BUILD-INFO.json"
    )
    $AllowedDirectories = @($Root, "${Root}LICENSES/")
    $Seen = @{}
    $Zip = [System.IO.Compression.ZipFile]::OpenRead($Archive)
    try {
        foreach ($Entry in $Zip.Entries) {
            $Name = $Entry.FullName.Replace("\", "/")
            if (-not $Name.StartsWith($Root, [System.StringComparison]::Ordinal) -or
                $Name -match '(^|/)\.\.($|/)' -or
                [System.IO.Path]::IsPathRooted($Name)) {
                throw "Archive contains a path outside ${Root}: $Name"
            }
            if ($Name -notin $RequiredFiles -and $Name -notin $AllowedDirectories) {
                throw "Archive payload does not match the release manifest: $Name"
            }
            $UnixType = (($Entry.ExternalAttributes -shr 16) -band 0xF000)
            $WindowsReparsePoint = ($Entry.ExternalAttributes -band 0x400) -ne 0
            if ($UnixType -eq 0xA000 -or $WindowsReparsePoint) {
                throw "Archive contains a link or reparse-point entry: $Name"
            }
            if ($Name -in $RequiredFiles) { $Seen[$Name] = $true }
        }
        foreach ($Required in $RequiredFiles) {
            if (-not $Seen.ContainsKey($Required)) {
                throw "Archive is missing required release file: $Required"
            }
        }
    } finally {
        $Zip.Dispose()
    }

    Expand-Archive -LiteralPath $Archive -DestinationPath $Temp -Force
    $Source = Join-Path $Temp "synapse-memory-$Target\synx.exe"
    if (-not (Test-Path -LiteralPath $Source -PathType Leaf)) {
        throw "Archive does not contain synapse-memory-$Target\synx.exe"
    }
    $Magic = [System.IO.File]::ReadAllBytes($Source)[0..1]
    if ($Magic[0] -ne 0x4d -or $Magic[1] -ne 0x5a) {
        throw "Archive does not contain a native Windows executable"
    }

    $BinDir = Join-Path $Prefix "bin"
    New-Item -ItemType Directory -Force -Path $BinDir | Out-Null
    $DatabaseDir = Split-Path -Parent $Database
    if ($DatabaseDir) { New-Item -ItemType Directory -Force -Path $DatabaseDir | Out-Null }
    $Destination = Join-Path $BinDir "synx.exe"
    if (Test-Path -LiteralPath $Destination) {
        Copy-Item -LiteralPath $Destination -Destination "$Destination.previous" -Force
    }
    Copy-Item -LiteralPath $Source -Destination $Destination -Force

    & $Destination -f $Database init | Out-Null
    & $Destination -f $Database doctor --json | Out-Null

    if (-not $NoPathUpdate) {
        $UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
        $Parts = @($UserPath -split ";" | Where-Object { $_ })
        if ($Parts -notcontains $BinDir) {
            [Environment]::SetEnvironmentVariable("Path", (($Parts + $BinDir) -join ";"), "User")
        }
        if (($env:Path -split ";") -notcontains $BinDir) { $env:Path += ";$BinDir" }
    }

    [pscustomobject]@{
        installed = $Destination
        database = $Database
        version = (& $Destination --version)
    } | ConvertTo-Json
} finally {
    Remove-Item -LiteralPath $Temp -Recurse -Force -ErrorAction SilentlyContinue
}
