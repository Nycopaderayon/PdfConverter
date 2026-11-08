$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$exe = Join-Path $root "target\debug\pdfword.exe"

# Pick up toolchains installed after this shell was opened (Rust + MinGW DLLs).
$env:Path = "$env:USERPROFILE\.cargo\bin;" +
    [Environment]::GetEnvironmentVariable("Path", "Machine") + ";" +
    [Environment]::GetEnvironmentVariable("Path", "User")

if (-not (Test-Path $exe)) {
    throw "Server binary not found at $exe. Run 'cargo build -p pdfword-web' first."
}

$env:PDFIUM_PATH = Join-Path $root "vendor\pdfium\bin"
$env:WORK_DIR = Join-Path $root "jobs"

$soffice = "C:\Program Files\LibreOffice\program\soffice.exe"
if (Test-Path $soffice) {
    $env:SOFFICE_PATH = $soffice
}

if (-not $env:BIND_ADDR) {
    $env:BIND_ADDR = "127.0.0.1:8080"
}

Write-Host "Starting pdfword on http://$env:BIND_ADDR"
& $exe
