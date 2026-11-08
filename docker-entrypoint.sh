#!/bin/sh
set -e

# Fonts may be mounted at runtime; refresh the cache before conversion so
# LibreOffice (and the PDF export filter) can see them.
if [ "$(id -u)" = "0" ]; then
  if [ -d /usr/share/fonts/truetype/custom ]; then
    fc-cache -f /usr/share/fonts/truetype/custom >/dev/null 2>&1 || true
  fi

  if command -v setpriv >/dev/null 2>&1; then
    exec setpriv --reuid=nobody --regid=nogroup --clear-groups "$@"
  fi
fi

exec "$@"
