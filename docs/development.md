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

### WPT conformance

`just test-wpt` runs 140 web-platform-tests files from `fetch/api` and `cors` inside miniclient
(about 1.5 minutes) and fails when a subtest that passed in `tests/wpt/expected.json` no longer
does. After a change that makes more subtests pass, record them with
`just update-wpt`. The files excluded from `tests/wpt/files.txt` need
XMLHttpRequest, iframes, workers or HTTP/2, which miniclient does not provide. The tests come from
`vendor/jsdom/test/web-platform-tests`, so that submodule must be checked out.

`just test-wpt-extended` runs a larger set (`files-extended.txt`: the rest of
`fetch/api`, `xhr`, form submission, DOM events, timers, `url` and `eventsource`) against
`expected-extended.json`. A single run takes about 20 minutes, so it is not part of `just test-wpt`.

`just test-wpt-css` runs the CSS files in `files-css.txt` (about 20 seconds) against
`expected-css.json`: every file jsdom runs from `cssom`, `css-cascade`, `css-conditional`,
`css-nesting`, `css-variables`, `css-syntax`, `css-values`, `css-animations` and `mediaqueries`, the
`parsing/` directories of the other CSS modules, and jsdom's `to-upstream/css` tests, served under
`/to-upstream/`. Left out: files jsdom skips as slow or timing out, `selectors` (selector matching),
`cssom-view` and `geometry` (layout), and the few files that hang until the per-file timeout. Every
run names the files that ended with no result (`no result: ...`), since they record no passes just
like a file that fails everything.
