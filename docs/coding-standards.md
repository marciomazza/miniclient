---
icon: lucide/ruler
---

# Coding Standards

## Language

Commit messages and code (identifiers, comments) are written in English.

## Type checking

The project uses [pyrefly](https://pyrefly.org/) as the Python type checker.

## Comments

A comment states the non-obvious WHY of the code as it is now, in 2-3 lines max. If there is
no non-obvious why, write no comment.

Never write:

- Provenance or history: where the code came from, what it replaces or ports, what it used to
  do ("port of X", "ported from", "replaces Y", "formerly Z", "same as the old ...").
  That belongs in the commit message.
- The investigation process ("turns out", "after debugging", "we found that").
- References to scratch/todo/spec/planning material — not the files, not their contents
  (e.g. "spec §4", ticket numbers, "see the research doc").

This holds even when only moving or rewriting an existing comment: strip the reference, never
carry it over. Every comment must stand on its own, readable by someone who never saw the
previous code.

## Avoid escaped quotes

Whatever the language (python, rust or javascript), prefer a quoting style that avoids escaped quotes over one that
needs them. In Python, this means triple-quoted strings for embedded code (JS, SQL, shell,
etc.) instead of escaping:

```python
runtime.eval(f"""\
  const html = "<div><p id='x'></p></div>"
""")
```

## Writing JS eval strings

DOM interaction is done by evaluating JavaScript inside the runtime via `runtime.eval()`.
Name the Python variable holding a JS code string `js`, or with a `_js` suffix (e.g.
`click_js`).
