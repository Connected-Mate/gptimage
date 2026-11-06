#!/usr/bin/env python3
"""Write the release tag into the docs that name the latest release.

  stamp_release.py <tag>          e.g. v0.3.7: rewrite the README's latest-release line and the reference's archive names
  stamp_release.py --check <tag>  exit 1 when a file still names another release

Run by the release workflow after a tag is published, so nobody edits these lines by hand. The README's line sits
between <!-- latest-release --> markers. The reference's install table names each archive as hoi4-imagegen-vX.Y.Z-<target>.
"""
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
RULES = [
    ("README.md", re.compile(r"(<!-- latest-release -->)v\d+\.\d+\.\d+(<!-- /latest-release -->)"), r"\g<1>{tag}\g<2>"),
    ("docs/reference.md", re.compile(r"(hoi4-imagegen-)v\d+\.\d+\.\d+(-[a-z0-9_]+-[a-z0-9_-]+\.(?:zip|tar\.gz))"), r"\g<1>{tag}\g<2>"),
]


def main(args):
    check = args[:1] == ["--check"]
    tag = args[-1]
    if not re.fullmatch(r"v\d+\.\d+\.\d+", tag):
        sys.exit(f"not a release tag: {tag}")
    stale = []
    for rel, pattern, repl in RULES:
        path = os.path.join(ROOT, rel)
        text = open(path, encoding="utf-8").read()
        if not pattern.search(text):
            sys.exit(f"{rel}: no release line to stamp")
        new = pattern.sub(repl.format(tag=tag), text)
        if new != text:
            stale.append(rel)
            if not check:
                with open(path, "w", encoding="utf-8", newline="\n") as f:
                    f.write(new)
    print(("stale: " if check else "stamped: ") + (", ".join(stale) or "nothing"))
    if check and stale:
        sys.exit(1)


if __name__ == "__main__":
    main(sys.argv[1:])
