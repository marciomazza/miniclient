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
./scripts/setup-vendor-wpt.sh    # clones web-platform-tests into vendor/wpt, only for `just test-wpt`
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

### WPT conformance

`just test-wpt` runs 140 web-platform-tests files from `fetch/api` and `cors` inside miniclient
(about 1.5 minutes) and fails when a subtest that passed in `tests/wpt/expected.json` no longer
does. After a change that makes more subtests pass, record them with
`uv run python tests/wpt/run.py --update`. The files excluded from `tests/wpt/files.txt` need
XMLHttpRequest, iframes, workers or HTTP/2, which miniclient does not provide.

`uv run python tests/wpt/run.py --extended` runs a larger set (`files-extended.txt`: the rest of
`fetch/api`, `xhr`, form submission, DOM events, timers, `url` and `eventsource`) against
`expected-extended.json`. A single run takes about 20 minutes, so it is not part of `just test-wpt`.
