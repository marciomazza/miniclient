#!/usr/bin/env bash
# Clones bigskysoftware/htmx at a branch into vendor/htmx for local dev/testing.
set -euo pipefail

REF="four-dev"
REPO="git@github.com:bigskysoftware/htmx.git"
DEST="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/vendor/htmx"

rm -rf "$DEST"
git clone --branch "$REF" --depth 1 "$REPO" "$DEST"
