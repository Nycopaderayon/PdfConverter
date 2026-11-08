param(
    [string]$Version = "latest",
    [string]$Dest = (Join-Path $PSScriptRoot "..\vendor\pdfium")
)

$ErrorActionPreference = "Stop"

$url = if ($Version -eq "latest") {
    "https://github.com/bblanchon/pdfium-binaries/releases/latest/download/pdfium-win-x64.tgz"
} else {
    "https://github.com/bblanchon/pdfium-binaries/releases/download/$Version/pdfium-win-x64.tgz"
}

New-Item -ItemType Directory -Force -Path $Dest | Out-Null

$archive = Join-Path $env:TEMP "pdfium-win-x64.tgz"
Write-Host "Downloading $url"
Invoke-WebRequest -Uri $url -OutFile $archive

tar -xzf $archive -C $Dest
Remove-Item $archive

Write-Host "Pdfium extracted to $Dest"
Write-Host "Set PDFIUM_PATH to: $(Join-Path $Dest 'bin')"
