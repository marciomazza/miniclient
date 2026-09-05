#!/usr/bin/env bash
# Points vendor/happy-dom at marciomazza/happy-dom (integration branch "join") and builds
# packages/happy-dom, which package.json's "happy-dom" dependency reads from directly.
#
# Locally this is a real, persistent dev clone -- rerunning must not clobber it. In CI it's
# disposable: always fetched fresh, pinned to a commit so a later push to "join" can't
# silently change what CI builds against.
set -euo pipefail

REF="f111bc58bcd5556cff29ba1fdc5d516864022f8c"  # join 2026-09-05
REPO="git@github.com:marciomazza/happy-dom.git"
DEST="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/vendor/happy-dom"

if [[ "${CI:-}" == "true" ]]; then
  # Don't rm -rf: a cache-restore step may have already placed packages/happy-dom/lib
  # here before this script runs (see ci.yml), and the tarball has no lib/ of its own
  # (build output isn't tracked in git) to conflict with it.
  mkdir -p "$DEST"
  curl -fsSL "https://github.com/marciomazza/happy-dom/archive/${REF}.tar.gz" \
    | tar -xz -C "$DEST" --strip-components=1
elif [[ ! -e "$DEST" ]]; then
  git clone "$REPO" "$DEST"
  git -C "$DEST" checkout -q "$REF"
else
  echo "vendor/happy-dom already exists, leaving it as-is (it's your own dev clone)"
fi

if [[ ! -d "$DEST/packages/happy-dom/lib" ]]; then
  ( cd "$DEST" && npm ci --silent && npx turbo run compile --filter=happy-dom )
fi
