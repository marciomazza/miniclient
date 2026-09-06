# Rebuild the native extension into the venv, picking up Rust / JS / vendor/happy-dom changes.
develop:
    uvx maturin develop --uv

# Rust tests only -- no venv rebuild needed.
test-rust:
    cargo nextest run

# Python tests -- rebuilds the extension first so the bundled happy-dom is current.
test-py: develop
    uv run pytest -n4

# Full suite.
test: test-rust test-py
