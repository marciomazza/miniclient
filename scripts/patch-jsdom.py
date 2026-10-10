#!/usr/bin/env python3
"""Regenerates patches/ from the two repos that track the jsdom fork's changes.

vendor/jsdom tracks the hand-written files against its `v30.1.2` tag; vendor/jsdom/lib/generated
is a repo of its own (the main one ignores it) tracked against its `published-30.1.2` bookmark.
The published package ships no .webidl sources, hence the exclusion.
"""

import re
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FORK = ROOT / "vendor/jsdom"
PREFIX = "node_modules/jsdom/"


def jj_diff(repo: Path, base: str, *paths: str) -> str:
    cmd = ["jj", "diff", "--no-pager", "--git", "--from", base, "--to", "@", *paths]
    return subprocess.run(cmd, cwd=repo, check=True, capture_output=True, text=True).stdout


def head(repo: Path) -> str:
    # @ is usually an empty working-copy change; name the last one that holds content.
    # A clean merge is "empty" to jj yet is the state the patch came from, so keep merges.
    rev = "heads(::@ ~ (empty() ~ merges()))"
    tpl = 'change_id.short() ++ " " ++ commit_id.short()'
    cmd = ["jj", "log", "--no-graph", "--no-pager", "-r", rev, "-T", tpl]
    return subprocess.run(cmd, cwd=repo, check=True, capture_output=True, text=True).stdout


def prefixed(diff: str, prefix: str) -> str:
    def fix(line: str) -> str:
        if line.startswith(("diff --git ", "--- ", "+++ ")):
            return re.sub(r"(?<!\S)([ab])/", rf"\1/{prefix}", line)
        return line

    return "\n".join(map(fix, diff.split("\n")))


def main():
    if not FORK.exists():  # fresh clone or CI: nothing to regenerate from
        return
    version = "30.1.2"
    fork = jj_diff(FORK, f"v{version}", 'lib ~ glob:"**/*.webidl"')
    generated = jj_diff(FORK / "lib/generated", f"published-{version}")
    for old in (ROOT / "patches").glob("jsdom+*.patch"):
        old.unlink()
    for name, repo, diff, prefix in (
        ("001+fork", FORK, fork, PREFIX),
        ("002+generated", FORK / "lib/generated", generated, PREFIX + "lib/generated/"),
    ):
        header = f"# fork: {head(repo)}\n"
        (ROOT / f"patches/jsdom+{version}+{name}.patch").write_text(header + prefixed(diff, prefix))


main()
