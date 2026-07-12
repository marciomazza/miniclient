---
icon: lucide/download
---
# Development

This page covers everything needed to work on this project locally: installing dependencies,
linting, and running the test suite.

## Project setup

Install dependencies and vendor htmx:

```bash
uv sync
npm install
./scripts/setup-vendor-htmx.sh   # clones htmx source into vendor/htmx, required by the tests
```

## Linting

We use [prek](https://prek.j178.dev) for linting and formatting.

To install the git hooks (so they run automatically on each commit):

```bash
prek install
```

To run all hooks manually (auto-fixes most issues in place):

```bash
prek -a
```

## Testing

The test suite includes some [hypothesis](https://hypothesis.readthedocs.io/) property-based tests
that crosscheck the behavior of this client against a real browser in many different scenarios.
This part of the tests can be rather slow so it's disabled by default.

To run all tests but the crosscheck:

```bash
pytest
```

To run only the hypothesis crosscheck tests:

```bash
pytest -m "cross"
```

To run the full suite, including the crosscheck tests:

```bash
pytest -m ""
```

Tests can run in parallel by adding the `pytest` option `-n auto`.
