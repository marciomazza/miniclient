#!/usr/bin/env bash
# Clones bigskysoftware/htmx at a pinned tag into vendor/htmx for local dev/testing.
set -euo pipefail

TAG="v4.0.0-beta5"
REPO="git@github.com:bigskysoftware/htmx.git"
DEST="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/vendor/htmx"

rm -rf "$DEST"
git clone --branch "$TAG" --depth 1 "$REPO" "$DEST"
