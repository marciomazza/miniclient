#!/usr/bin/env bash
# Clones web-platform-tests at a pinned commit into vendor/wpt, keeping only the directories the
# fetch/CORS suite in tests/wpt needs (the full checkout is over 1 GB).
set -euo pipefail

REF="da1355b3fc629f97f8194884ab113da1ba3c7990"  # master 2026-09
REPO="https://github.com/web-platform-tests/wpt.git"
DEST="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/vendor/wpt"

rm -rf "$DEST"
git clone --quiet --filter=blob:none --no-checkout "$REPO" "$DEST"
git -C "$DEST" sparse-checkout set tools resources common fetch cors docs xhr/resources
git -C "$DEST" checkout --quiet "$REF"
