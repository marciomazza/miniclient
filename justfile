# Rebuild the native extension into the venv, picking up Rust / JS / jsdom-bundle changes.
develop:
    uvx maturin develop --uv

# Rust tests only -- no venv rebuild needed.
test-rust:
    cargo nextest run

# Python tests -- rebuilds the extension first so the bundled jsdom is current.
test-py: develop
    uv run pytest -n4

# Full suite.
test: test-rust test-py

# WPT fetch/CORS suite against the built extension; fails on regressions from tests/wpt/expected.json.
test-wpt: develop
    uv run python tests/wpt/run.py

# Regenerate patches/ from vendor/jsdom, then reinstall so node_modules matches.
# Each patch's header names the fork change it came from.
sync-jsdom:
    python3 scripts/patch-jsdom.py
    npm ci
    npx patch-package --error-on-fail

# Type-check the JS under js/ (files opt in with // @ts-check).
check-js:
    npx tsc -p jsconfig.json
