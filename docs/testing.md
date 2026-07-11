---
icon: lucide/flask-conical
---

# Testing

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
