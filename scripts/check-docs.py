#!/usr/bin/env python3
"""Check btrn agent guidance and specs for broken links and dead path rules.

Checks:
  * every relative Markdown link and heading anchor in AGENTS.md, CLAUDE.md,
    docs/, and .github/ resolves;
  * every applyTo pattern in .github/instructions matches a git-tracked file;
  * every path pattern in the AGENTS.md "Subsystem rules" table matches a
    git-tracked file.

Exits non-zero and prints each problem. Uses only the standard library.
"""

import os
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
LINK = re.compile(r"\[[^\]]*\]\(([^)\s]+)\)")
FENCE = re.compile(r"^(```|~~~)")


def markdown_files():
    files = [ROOT / "AGENTS.md", ROOT / "CLAUDE.md"]
    files += sorted((ROOT / "docs").rglob("*.md"))
    files += sorted((ROOT / ".github").rglob("*.md"))
    return [path for path in files if path.is_file()]


def prose_lines(path):
    """Lines outside fenced code blocks."""
    inside = False
    for line in path.read_text(encoding="utf-8").splitlines():
        if FENCE.match(line.strip()):
            inside = not inside
            continue
        if not inside:
            yield line


def slug(heading):
    """GitHub's heading anchor: lowercase, punctuation dropped, spaces to hyphens."""
    text = re.sub(r"[`*_]", "", heading.strip().lower())
    text = re.sub(r"[^\w\- ]", "", text)
    return text.replace(" ", "-")


def anchors(path):
    found = set()
    for line in prose_lines(path):
        match = re.match(r"#{1,6}\s+(.*)", line)
        if match:
            found.add(slug(match.group(1)))
    return found


def check_links(problems):
    for path in markdown_files():
        for line in prose_lines(path):
            for target in LINK.findall(line):
                if re.match(r"[a-z]+:", target):
                    continue
                file_part, _, anchor = target.partition("#")
                dest = (path.parent / file_part).resolve() if file_part else path
                where = path.relative_to(ROOT)
                if not dest.exists():
                    problems.append(f"{where}: link target missing: {target}")
                elif anchor and dest.suffix == ".md" and anchor not in anchors(dest):
                    problems.append(f"{where}: anchor missing: {target}")


def tracked_files():
    output = subprocess.run(
        ["git", "ls-files", "--cached", "--others", "--exclude-standard"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    return output.split()


def glob_regex(pattern):
    """Translate a glob where ** spans directories and * stays within one."""
    out = ""
    i = 0
    while i < len(pattern):
        if pattern.startswith("**/", i):
            out += "(?:.*/)?"
            i += 3
        elif pattern.startswith("**", i):
            out += ".*"
            i += 2
        elif pattern[i] == "*":
            out += "[^/]*"
            i += 1
        elif pattern[i] == "?":
            out += "[^/]"
            i += 1
        else:
            out += re.escape(pattern[i])
            i += 1
    return re.compile(out + r"\Z")


def check_patterns(problems):
    files = tracked_files()

    def matches(pattern):
        regex = glob_regex(pattern)
        return any(regex.match(name) for name in files)

    for path in sorted((ROOT / ".github" / "instructions").glob("*.md")):
        text = path.read_text(encoding="utf-8")
        front = text.split("---")[1] if text.startswith("---") else ""
        for pattern in re.findall(r'^\s*-\s*"([^"]+)"', front, re.M):
            if not matches(pattern):
                problems.append(
                    f"{path.relative_to(ROOT)}: applyTo matches nothing: {pattern}"
                )

    agents = (ROOT / "AGENTS.md").read_text(encoding="utf-8")
    section = agents.split("## Subsystem rules", 1)[-1].split("\n## ", 1)[0]
    for row in section.splitlines():
        if not row.startswith("|"):
            continue
        first_cell = row.split("|")[1]
        for pattern in re.findall(r"`([^`]+)`", first_cell):
            if not matches(pattern):
                problems.append(f"AGENTS.md: table path matches nothing: {pattern}")


def main():
    os.chdir(ROOT)
    problems = []
    check_links(problems)
    check_patterns(problems)
    for problem in problems:
        print(problem)
    if problems:
        print(f"{len(problems)} problem(s)", file=sys.stderr)
        return 1
    print("docs OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
