param(
    [Parameter(Mandatory = $true)]
    [ValidateSet("1", "2", "3", "restore", "status")]
    [string]$Stage
)

$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSScriptRoot
$media = Join-Path $root "Game\Darksiders II Deathinitive Edition\media"
$exe = Join-Path $root "target\release\ds2-modkit.exe"
$manifest = Join-Path $media "scripts.obsp"
$backup = Join-Path $media "scripts.obsp.bak"
$master = Join-Path $media "_modkit_work\scripts.stock.obsp"
$stockSize = 18334463

function Get-State {
    $size = (Get-Item $manifest).Length
    if ($size -ne $stockSize) { return "MODIFIED" }
    if ((Get-FileHash $manifest -Algorithm SHA256).Hash -ne (Get-FileHash $master -Algorithm SHA256).Hash) {
        return "MODIFIED (same size)"
    }
    return "stock"
}

function Restore-Stock {
    $source = if (Test-Path $master) { $master } elseif (Test-Path $backup) { $backup } else { $null }
    if (-not $source) { throw "no master copy or backup to restore from" }
    Copy-Item $source $manifest -Force -ErrorAction Stop
    if (Test-Path $backup) { Remove-Item $backup -Force -ErrorAction Stop }
    "restored stock scripts.obsp from $source ($((Get-Item $manifest).Length) bytes)"
}

switch ($Stage) {
    "status" {
        "scripts.obsp: $((Get-Item $manifest).Length) bytes ($(Get-State))"
        if (Test-Path $master) { "master copy: present ($master)" } else { "master copy: MISSING" }
        if (Test-Path $backup) { "backup: present ($backup)" }
    }
    "restore" { Restore-Stock }
    default {
        if (-not (Test-Path $exe)) { throw "build first: cargo build --release" }
        Restore-Stock | Out-Null
        $folder = @{ "1" = "verify-1-values"; "2" = "verify-2-structural"; "3" = "verify-3-import" }[$Stage]
        & $exe apply (Join-Path $root "mods\$folder\mod.toml")
        if ($LASTEXITCODE -ne 0) {
            throw "apply failed with exit code $LASTEXITCODE; scripts.obsp was not left modified by this run (check game is closed)"
        }
        "stage $Stage applied: scripts.obsp $((Get-Item $manifest).Length) bytes ($(Get-State)). Restore: tools\verify-in-game.ps1 restore"
    }
}
