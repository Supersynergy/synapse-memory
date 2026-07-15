param(
    [string]$Prefix = "$env:LOCALAPPDATA\Synapse",
    [string]$Database = "$env:USERPROFILE\.synapse\brain.db",
    [switch]$PurgeData
)

$ErrorActionPreference = "Stop"
$BinDir = Join-Path $Prefix "bin"
Remove-Item -LiteralPath (Join-Path $BinDir "synx.exe") -Force -ErrorAction SilentlyContinue

$UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
$CleanPath = (($UserPath -split ";" | Where-Object { $_ -and $_ -ne $BinDir }) -join ";")
[Environment]::SetEnvironmentVariable("Path", $CleanPath, "User")

if ($PurgeData) {
    Remove-Item -LiteralPath $Database, "$Database-wal", "$Database-shm" -Force -ErrorAction SilentlyContinue
    Write-Output "removed binary and data=$Database"
} else {
    Write-Output "removed binary; preserved data=$Database"
}
