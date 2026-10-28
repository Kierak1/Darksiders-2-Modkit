# AGENTS.md section 1 verification: run the whole table, exit 0 = all green.
# Usage (from the workspace root): powershell -File tools/check.ps1
$ErrorActionPreference = 'Continue'
Push-Location -LiteralPath (Join-Path $PSScriptRoot '..')

$failed = @()
function Step {
    param([string]$Name, [scriptblock]$Command)
    Write-Host "== $Name ==" -ForegroundColor Cyan
    & $Command
    if ($LASTEXITCODE -ne 0) {
        $script:failed += $Name
        Write-Host "FAILED: $Name (exit $LASTEXITCODE)" -ForegroundColor Red
    }
}

Step 'cargo check'  { cargo check --workspace --all-targets }
Step 'cargo fmt'    { cargo fmt --all -- --check }
Step 'cargo clippy' { cargo clippy --workspace --all-targets --all-features -- -D warnings }
Step 'cargo test'   { cargo test --workspace --all-features }
Step 'cargo doc'    { cargo doc --workspace --no-deps }

Pop-Location
if ($failed.Count -gt 0) {
    Write-Host "check FAILED: $($failed -join ', ')" -ForegroundColor Red
    exit 1
}
Write-Host "check passed: check, fmt, clippy, test, doc" -ForegroundColor Green
exit 0
