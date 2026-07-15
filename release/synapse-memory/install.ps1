param(
    [string]$Version = "",
    [string]$Prefix = "$env:LOCALAPPDATA\Synapse",
    [string]$Database = "$env:USERPROFILE\.synapse\brain.db",
    [switch]$DryRun,
    [switch]$NoPathUpdate
)

$ErrorActionPreference = "Stop"
$Url = "https://raw.githubusercontent.com/Supersynergy/synapse-agent-memory/main/release/synapse-agent-memory/install.ps1"
$Temp = Join-Path ([System.IO.Path]::GetTempPath()) ("synapse-agent-memory-forward-" + [guid]::NewGuid() + ".ps1")

try {
    Invoke-WebRequest -UseBasicParsing $Url -OutFile $Temp
    & $Temp @PSBoundParameters
} finally {
    Remove-Item -LiteralPath $Temp -Force -ErrorAction SilentlyContinue
}
