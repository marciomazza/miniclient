#!/usr/bin/env bash
# Clones bigskysoftware/htmx at a branch into vendor/htmx for local dev/testing.
set -euo pipefail

REF="a689089e"  # four-dev 2026-08-21
REPO="git@github.com:bigskysoftware/htmx.git"
DEST="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/vendor/htmx"

rm -rf "$DEST"
if [[ "${CI:-}" == "true" ]]; then
  mkdir -p "$DEST"
  curl -sL "https://github.com/bigskysoftware/htmx/archive/${REF}.tar.gz" \
    | tar -xz -C "$DEST" --strip-components=1
else
  git clone "$REPO" "$DEST"
  git -C "$DEST" checkout -q "$REF"
fi
