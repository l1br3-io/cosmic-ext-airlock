#!/usr/bin/env python3
"""Link/asset integrity gate for the site (`just site-check`).

Parses every .html file under the directory given as argv[1] and verifies:

- every href/src that is not an external URL (http(s), mailto,
  protocol-relative) or fragment-only resolves to an existing file inside
  that directory, joined against the page's own location — a relative path
  may not escape the root, because Pages serves under /cosmic-ext-airlock/
  and the same rule keeps a local file:// preview working;
- every fragment (#x) names an id="x" on its target page — the page the
  link points at, or the page itself for fragment-only links;
- every <img> carries alt and explicit width/height (no reflow while
  images load);
- no page references raw.githubusercontent.com or an absolute path (/…)
  for a site asset — the site is self-contained, assets are copies.

Exit 0 with no output when clean. On any broken reference, prints one
`file:line → target (reason)` line each — paths relative to the site
root — and exits 1. A missing root directory fails loudly on its own, so
`just site-check` is honest before the site exists.
"""

import os
import re
import sys
from pathlib import Path

ARROW = "→"
# A tag opened on a line without its closing > (hand-wrapped attribute
# lists, like the spec's two-line <img>): joined with the following lines,
# keeping the line where the tag starts.
UNCLOSED = re.compile(r"<[a-zA-Z/][^>]*$")
IMG = re.compile(r"<img\b[^>]*>", re.IGNORECASE)
HREF_SRC = re.compile(r"""\b(href|src)\s*=\s*(?:"([^"]*)"|'([^']*)')""")
ATTRS = re.compile(r"""\b(href|src|alt|width|height|id)\s*=\s*(?:"([^"]*)"|'([^']*)')""")
EXTERNAL = ("http://", "https://", "mailto:", "//")


def logical_lines(text):
    """Yield (1-based line, text), with wrapped tags joined back together."""
    lines = text.splitlines()
    out = []
    i = 0
    while i < len(lines):
        line = lines[i]
        start = i
        while UNCLOSED.search(line) and i + 1 < len(lines):
            i += 1
            line += " " + lines[i]
        out.append((start + 1, line))
        i += 1
    return out


def problems_for_file(path, root):
    """Check one page; return (refs, img_tags, ids, problems).

    refs is [(line, target)] for the second, cross-file pass — fragment
    targets need the whole site's ids collected first.
    """
    rel = path.relative_to(root).as_posix()
    refs = []
    img_tags = []
    ids = set()
    problems = []

    def problem(line, target, reason):
        problems.append(f"{rel}:{line} {ARROW} {target} ({reason})")

    text = path.read_text(encoding="utf-8", errors="replace")
    for line, buf in logical_lines(text):
        for m in ATTRS.finditer(buf):
            if m.group(1) == "id":
                ids.add(m.group(2) or m.group(3))
        for m in HREF_SRC.finditer(buf):
            refs.append((line, m.group(2) or m.group(3)))
        for m in IMG.finditer(buf):
            img_tags.append((line, m.group(0)))
    return rel, refs, img_tags, ids, problems


def check_reference(target, page_rel, page_dir, root, ids_by_page):
    """Resolve one href/src target; return (line_loc_suffix, reason) or None."""
    if "raw.githubusercontent.com" in target:
        return "raw.githubusercontent.com asset — assets are self-contained copies"
    if target.startswith(EXTERNAL):
        return None
    if target.startswith("#"):
        if target[1:] not in ids_by_page[page_rel]:
            return "no such id on this page"
        return None
    if target.startswith("/"):
        return "absolute path — site assets are relative under the Pages subpath"

    path_part, _, frag = target.partition("#")
    if not path_part:
        return "empty reference"
    # Lexical normalization, not resolve(): the rule is that the path text
    # stays inside the site root, symlinks aside.
    dest = os.path.normpath(os.path.join(page_dir, path_part))
    root_norm = os.path.normpath(str(root))
    if not (dest == root_norm or dest.startswith(root_norm + os.sep)):
        return "escapes the site directory"
    if not os.path.isfile(dest):
        return "no such file"
    if frag:
        dest_rel = Path(dest).relative_to(root_norm).as_posix()
        if frag not in ids_by_page.get(dest_rel, set()):
            return f"no id '{frag}' on {dest_rel}"
    return None


def main():
    try:
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    except AttributeError:
        pass
    if len(sys.argv) != 2:
        print(f"check-site-links: usage: {Path(sys.argv[0]).name} <site-dir>",
              file=sys.stderr)
        return 1
    root = Path(sys.argv[1])
    if not root.is_dir():
        print(f"check-site-links: {root}: no such directory", file=sys.stderr)
        return 1

    pages = sorted(root.rglob("*.html"))
    if not pages:
        print(f"check-site-links: {root}: no .html pages found", file=sys.stderr)
        return 1

    problems = []
    deferred = []  # (rel, line, target)
    ids_by_page = {}
    dirs_by_page = {}
    file_problems = {}
    for path in pages:
        rel, refs, img_tags, ids, probs = problems_for_file(path, root)
        ids_by_page[rel] = ids
        dirs_by_page[rel] = str(path.parent)
        file_problems[rel] = probs
        for line, buf_img in img_tags:
            attrs = {}
            for m in ATTRS.finditer(buf_img):
                attrs.setdefault(m.group(1), m.group(2) or m.group(3))
            src = attrs.get("src")
            if "alt" not in attrs:
                file_problems[rel].append(
                    f"{rel}:{line} {ARROW} {src or '<img>'} (img missing alt)")
            if "width" not in attrs or "height" not in attrs:
                file_problems[rel].append(
                    f"{rel}:{line} {ARROW} {src or '<img>'} (img missing explicit width/height)")
        for line, target in refs:
            deferred.append((rel, line, target))

    for rel, line, target in deferred:
        reason = check_reference(target, rel, dirs_by_page[rel], root, ids_by_page)
        if reason:
            file_problems[rel].append(f"{rel}:{line} {ARROW} {target} ({reason})")

    for rel in sorted(file_problems):
        problems.extend(file_problems[rel])
    if problems:
        for p in problems:
            print(p)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
