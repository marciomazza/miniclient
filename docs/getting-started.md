---
icon: lucide/download
---

# Getting started

## Bootstrap the project environment

```bash
uv sync
npm install
./scripts/setup-vendor-htmx.sh   # clones htmx source into vendor/htmx, required by the tests
```

## Running the tests

```bash
pytest
```

The test suite includes some hypothesis property-based tests that crosscheck the behavior of this
client against a real browser in many different scenarios. This part of the tests can be rather
slow so it's disabled by default — see [Testing](testing.md) for how to run it.

Tests can run in parallel by adding the `pytest` option `-n auto`.
