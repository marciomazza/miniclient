# Project Instructions

All code, docs and jj describe messages must be written in English.

# Architecture

## Runtime stack

- **jsrun** (V8 via deno_core + PyO3 bindings) is the JavaScript runtime. It is NOT Node.js and NOT QuickJS.
- **happy-dom** runs INSIDE jsrun, loaded with custom module polyfills for Node modules (`node:buffer`, `node:stream`, `node:crypto`, etc.).
- **htmx** runs inside the same jsrun context, initialized after happy-dom's `Window` is set up. It uses the polyfilled `fetch` and timers. `fetch` is done via `httpx`
- The Python `Browser` class in `src/htmxclient/browser.py` wraps a jsrun `Runtime`. It has NO relation to happy-dom's `Browser` class.

## DOM interaction

All DOM interaction (click, submit, dispatchEvent, query selectors, etc.) is done by evaluating JavaScript inside the jsrun runtime via `runtime.eval()` / `runtime.eval_async()`. There is no direct Python API to happy-dom or htmx objects — they live inside the V8 isolate.

When adding DOM interaction methods, implement them in Python using `runtime.eval()` / `runtime.eval_async()` to execute JS against the happy-dom `document` already initialized in `bootstrap.js`.

# Process

Always plan before execution. Show your plan and ask for confirmation.
When in doubt ask clarification questions.
Always ask for confirmation before changing existing code, unless told otherwise.
If instead of confirming the user continues the conversation, do not assume the change is confirmed:
only after the theme is clarified ask for confirmation again, and only proceed with a clear confirmation.

# Version control on checkpoints

This project uses jujutsu as a VCS. Do not issue any git command.
Instead use jj equivalents, like `jj describe` and `jj new` when needed.

In each new session, before changing anything: if you are not in an empty change, create a new one with `jj new`.
Implement features in separate changes.
After each cohesive feature change:
  - make a `jj describe` for what you implemented
  - run `prek -a` to ensure all fixes are applied and linter rules are satisfied.
    Fix whatever is needed and rerun prek until there are no errors
  - run `jj new` to continue to the next feature change.

Never make a `git commit`, only use jujutsu.
When I say `commit` make a `jj describe` + `jj new`.

Never squash or rebase changes.

# Testing

Always add tests to the features you implement.
Use pytest writing simple functions.
Use `@pytest.mark.parametrize` for variations of the same behavior.
