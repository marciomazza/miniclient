# Project Instructions

All code, docs and jj describe messages must be written in English.

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
