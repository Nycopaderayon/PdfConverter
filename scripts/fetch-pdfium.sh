#!/usr/bin/env bash
set -euo pipefail

version="${PDFIUM_VERSION:-latest}"
dest="${1:-vendor/pdfium}"

if [ "$version" = "latest" ]; then
  url="https://github.com/bblanchon/pdfium-binaries/releases/latest/download/pdfium-linux-x64.tgz"
else
  url="https://github.com/bblanchon/pdfium-binaries/releases/download/${version}/pdfium-linux-x64.tgz"
fi

mkdir -p "$dest"
echo "Downloading $url"
curl -fsSL "$url" | tar -xz -C "$dest"

echo "Pdfium extracted to $dest"
echo "Set PDFIUM_PATH to: $dest/lib"
