# Project Instructions

All code, docs and jj describe messages must be written in English.

# Process

Always plan before execution. Show your plan and ask for confirmation.
When in doubt ask clarification questions.
Always ask for confirmation before changing existing code, unless told otherwise.

# Version control on checkpoints

When you reach a cohesive major change that deserves a checkpoint commit,
make a `jj describe` for what you implemented and `jj new` to continue to the next major change.

Never make a `git commit` only use jujutsu.
When I say `commit` make a `jj describe` + `jj new`.

Never squash or rebase changes.

# Testing

Use pytest writing simple functions.
Use `@pytest.mark.parametrize` for variations of the same behavior.
