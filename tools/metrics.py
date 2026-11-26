#!/usr/bin/env python3
"""Code size and static complexity report (not a gate: it never fails on what it measures).

Size: code, comment and blank lines per language over `git ls-files` (vendor/ apart), and Rust code lines per area,
with `#[cfg(test)]` code counted separately. Vendored Rust: all of it, and only the files the build compiles, read
from cargo's dep-info files. Complexity: clippy's `cognitive_complexity` and `too_many_lines` with thresholds of 0, so
they report nearly every function and closure. Both clippy runs (default features and `gui`, all targets, host) use
their own target dir and clippy.toml under target/metrics, so `just lint`'s caches are untouched.

The full result goes to target/metrics/last.json; the report ends with the change since the previous one. `--json`
prints that JSON instead of the tables.

Approximations: a line is a comment only if it holds nothing but comments (code with a trailing comment is code);
block comments opened after code on a line, nested Rust block comments and comment markers inside strings are not
seen; shebangs count as comments, Python docstrings as code, Markdown text as code. Test code is the item after each
`#[cfg(test)]`, delimited by counting braces on code lines (braces in string or char literals would skew it), plus
tests/. HTML's inline <script> and <style> count as code. A closure is named after the nearest `fn` above it. Only
code compiled for the host is ranked. cc 0 means clippy scored it 0 or less (it subtracts returns) and did not report
it; closures get no length; a function reported by neither lint is not listed.
"""

import json
import os
import re
import subprocess
import sys
import textwrap
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "target/metrics"
SNAPSHOT = OUT / "last.json"
TOP = 10

C_LIKE = (("//",), ("/*", "*/"))
HASH = (("#",), None)
MARKUP = ((), ("<!--", "-->"))
SYNTAX: dict[str, tuple[tuple[str, ...], tuple[str, str] | None]] = {
    "Rust": C_LIKE,
    "Python": HASH,
    "shell": HASH,
    "PowerShell": (("#",), ("<#", "#>")),
    "JavaScript": C_LIKE,
    "HTML": MARKUP,
    "CSS": C_LIKE,
    "YAML": HASH,
    "TOML": HASH,
    "justfile": HASH,
    "Markdown": MARKUP,
}
EXTENSIONS = {
    ".rs": "Rust",
    ".py": "Python",
    ".sh": "shell",
    ".ps1": "PowerShell",
    ".js": "JavaScript",
    ".html": "HTML",
    ".css": "CSS",
    ".yml": "YAML",
    ".yaml": "YAML",
    ".toml": "TOML",
    ".md": "Markdown",
}
AREAS = [
    ("src/frontend/gui", ("src/frontend/gui/",)),
    ("src/frontend", ("src/frontend/",)),
    ("src/backend", ("src/backend/",)),
    ("src/platform", ("src/platform/",)),
    ("src/main+protocol", ("src/main.rs", "src/protocol.rs")),
    ("crates/core", ("crates/core/",)),
    ("web/src", ("web/src/",)),
    ("tests", ("tests/",)),
]
LINTS = {"clippy::cognitive_complexity": "cc", "clippy::too_many_lines": "lines"}
CLIPPY_TOML = "cognitive-complexity-threshold = 0\ntoo-many-lines-threshold = 0\n"
FN = re.compile(r"\bfn\s+(\w+)")


def language(path: str) -> str | None:
    return "justfile" if path == "justfile" else EXTENSIONS.get(Path(path).suffix)


def classify(lines: list[str], lang: str) -> list[str]:
    markers, block = SYNTAX[lang]
    kinds = []
    in_block = False
    for raw in lines:
        s = raw.strip()
        kind = "comment" if s else "blank"
        while s:
            if in_block:
                end = s.find(block[1])
                if end < 0:
                    break
                in_block = False
                s = s[end + len(block[1]) :].lstrip()
            elif s.startswith(markers):
                break
            elif block and s.startswith(block[0]):
                in_block = True
                s = s[len(block[0]) :]
            else:
                kind = "code"
                break
        kinds.append(kind)
    return kinds


def test_lines(lines: list[str], kinds: list[str]) -> list[bool]:
    mask = [False] * len(lines)
    i = 0
    while i < len(lines):
        if lines[i].lstrip().startswith("#[cfg(test)]"):
            depth, j = 0, i
            for j in range(i, len(lines)):
                mask[j] = True
                if kinds[j] != "code":
                    continue
                depth += lines[j].count("{") - lines[j].count("}")
                rest = lines[j].replace("#[cfg(test)]", "").rstrip()
                if depth <= 0 and rest.endswith(("}", ";")):
                    break
            i = j
        i += 1
    return mask


def area(path: str) -> str:
    return next((name for name, prefixes in AREAS if path.startswith(prefixes)), "other")


def zero(*keys: str) -> dict[str, int]:
    return dict.fromkeys(keys, 0)


def measure_size(files: list[str]) -> tuple[dict, dict, dict, dict[str, list[bool]]]:
    langs: dict[str, dict[str, int]] = {}
    areas = {name: zero("code", "test") for name, _ in AREAS}
    vendor: dict[str, dict[str, int]] = {}
    tests: dict[str, list[bool]] = {}
    for path in files:
        lang = language(path)
        if lang is None or not (ROOT / path).is_file():
            continue
        lines = (ROOT / path).read_text(errors="replace").splitlines()
        kinds = classify(lines, lang)
        if path.startswith("vendor/"):
            if lang == "Rust":
                crate = vendor.setdefault(path.split("/")[1], zero("files", "code", "lines"))
                crate["files"] += 1
                crate["code"] += kinds.count("code")
                crate["lines"] += len(lines)
            continue
        counts = langs.setdefault(lang, zero("files", "code", "comment", "blank"))
        counts["files"] += 1
        for kind in kinds:
            counts[kind] += 1
        if lang == "Rust":
            mask = [True] * len(lines) if path.startswith("tests/") else test_lines(lines, kinds)
            tests[path] = mask
            row = areas.setdefault(area(path), zero("code", "test"))
            for kind, is_test in zip(kinds, mask):
                if kind == "code":
                    row["test" if is_test else "code"] += 1
    return langs, areas, vendor, tests


def clippy(features: list[str]) -> list[dict]:
    conf = OUT / "clippy-conf"
    conf.mkdir(parents=True, exist_ok=True)
    toml = conf / "clippy.toml"
    # Rewriting it unchanged would still bump its mtime and make clippy re-lint every workspace crate.
    if not toml.is_file() or toml.read_text() != CLIPPY_TOML:
        toml.write_text(CLIPPY_TOML)
    env = {**os.environ, "CARGO_TARGET_DIR": str(OUT), "CLIPPY_CONF_DIR": str(conf)}
    lints = [arg for lint in LINTS for arg in ("-W", lint)]
    cmd = ["cargo", "clippy", "-q", "--workspace", "--all-targets", *features, "--message-format=json"]
    proc = subprocess.run([*cmd, "--", "-A", "clippy::all", *lints], cwd=ROOT, env=env, capture_output=True, text=True)
    if proc.returncode != 0:
        sys.exit(f"metrics: {' '.join(cmd)} failed:\n{proc.stderr}")
    return [json.loads(line) for line in proc.stdout.splitlines() if line.startswith("{")]


def compiled_vendor_files(messages: list[dict]) -> set[str]:
    files = set()
    for m in messages:
        if m["reason"] != "compiler-artifact" or not Path(m["manifest_path"]).is_relative_to(ROOT / "vendor"):
            continue
        for artifact in map(Path, m["filenames"]):
            if artifact.parent.name == "deps":
                depinfo = [artifact.parent / (artifact.name.split(".")[0].removeprefix("lib") + ".d")]
            else:
                depinfo = list(artifact.parent.glob("*.d"))
            for d in depinfo:
                for line in d.read_text().splitlines():
                    if not line.startswith("#"):
                        files.update(line.partition(": ")[2].split())
    return {p for p in (Path(os.path.relpath(ROOT / f, ROOT)).as_posix() for f in files) if p.startswith("vendor/")}


def function_name(path: str, line: int) -> str:
    lines = (ROOT / path).read_text(errors="replace").splitlines()
    if m := FN.search(lines[line - 1]):
        return m.group(1)
    outer = next((m.group(1) for i in range(line - 2, -1, -1) if (m := FN.search(lines[i]))), "?")
    return f"{outer}::{{closure}}"


def functions(messages: list[dict], tests: dict[str, list[bool]]) -> list[dict]:
    reports = []
    for m in messages:
        msg = m.get("message") or {}
        lint = LINTS.get((msg.get("code") or {}).get("code") or "")
        span = next((s for s in msg.get("spans", []) if s["is_primary"]), None)
        if m["reason"] != "compiler-message" or lint is None or span is None:
            continue
        path = Path(span["file_name"]).as_posix()
        if path not in tests:
            continue
        value = re.search(r"\((\d+)/\d+\)", msg["message"])
        if value is None:
            sys.exit(f"metrics: unexpected clippy message: {msg['message']}")
        reports.append((path, span["line_start"], span["column_start"], lint, int(value.group(1))))
    found: dict[tuple[str, int], dict] = {}
    # too_many_lines spans a function from its first column, cognitive_complexity from `fn`, and a closure that starts
    # on its function's line reports later on it: per line, the leftmost cognitive_complexity is the function's. A
    # function compiled differently per feature or target keeps its highest value.
    for path, line, _, lint, value in sorted(reports, key=lambda r: (*r[:4], -r[4])):
        found.setdefault((path, line), {"file": path, "line": line}).setdefault(lint, value)
    result = list(found.values())
    seen: dict[str, int] = {}
    for fn in result:
        fn.setdefault("cc", 0)
        fn.setdefault("lines", 0)
        fn["name"] = function_name(fn["file"], fn["line"])
        fn["test"] = tests[fn["file"]][fn["line"] - 1]
        key = f"{fn['file']}:{fn['name']}"
        fn["id"] = key if key not in seen else f"{key}#{seen[key]}"
        seen[key] = seen.get(key, 0) + 1
    return result


def top(fns: list[dict], key: str) -> list[dict]:
    return sorted((f for f in fns if not f["test"]), key=lambda f: (-f[key], f["id"]))[:TOP]


def collect() -> dict:
    files = subprocess.run(["git", "ls-files"], cwd=ROOT, capture_output=True, text=True, check=True).stdout.split()
    langs, areas, vendor, tests = measure_size(files)
    messages = clippy([]) + clippy(["--features", "noiseman/gui"])
    compiled = compiled_vendor_files(messages)
    for name, crate in vendor.items():
        paths = [p for p in compiled if p.startswith(f"vendor/{name}/")]
        lines = [classify((ROOT / p).read_text(errors="replace").splitlines(), "Rust") for p in paths]
        crate.update(compiled_files=len(paths), compiled_code=sum(k.count("code") for k in lines))
        crate["compiled_lines"] = sum(map(len, lines))
    fns = functions(messages, tests)
    per_file: dict[str, dict[str, int]] = {}
    for fn in fns:
        row = per_file.setdefault(fn["file"], zero("cc", "lines"))
        row["cc"], row["lines"] = max(row["cc"], fn["cc"]), max(row["lines"], fn["lines"])
    langs = dict(sorted(langs.items(), key=lambda kv: -kv[1]["code"]))
    return {"time": time.strftime("%Y-%m-%d %H:%M:%S"), "languages": langs, "areas": areas, "vendor": vendor,
            "functions": fns, "files": per_file}


def table(title: str, header: list[str], rows: list[list[object]]) -> list[str]:
    cells = [header, *[[str(c) for c in row] for row in rows]]
    widths = [max(len(r[i]) for r in cells) for i in range(len(header))]
    just = [str.ljust, *[str.rjust] * (len(header) - 1)]
    return [title, *("  " + "  ".join(j(c, w) for j, c, w in zip(just, r, widths)) for r in cells)]


def trend(prev: dict, cur: dict) -> list[str]:
    def total(snap: dict, lang: str) -> int:
        return sum(v for k, v in snap["languages"].get(lang, {}).items() if k != "files")

    sizes = [f"{lang} {d:+d}" for lang in sorted({*prev["languages"], *cur["languages"]})
             if (d := total(cur, lang) - total(prev, lang))]
    old = {f["id"]: f for f in prev["functions"]}
    new = {f["id"]: f for f in cur["functions"]}
    old_top = {f["id"] for key in ("cc", "lines") for f in top(prev["functions"], key)}
    new_top = {f["id"] for key in ("cc", "lines") for f in top(cur["functions"], key)}
    changes = [f"+{i}" for i in sorted(new_top - old_top)] + [f"-{i}" for i in sorted(old_top - new_top)]
    for i in sorted(new_top & old.keys()):
        if new[i]["cc"] > old[i]["cc"] or new[i]["lines"] > old[i]["lines"]:
            changes.append(f"{i} cc {old[i]['cc']}->{new[i]['cc']} lines {old[i]['lines']}->{new[i]['lines']}")
    changes_text = "top functions: " + ("; ".join(changes) or "unchanged")
    return [f"since {prev['time']}: lines {', '.join(sizes) or 'unchanged'}",
            *textwrap.wrap(changes_text, 118, initial_indent="  ", subsequent_indent="    ", max_lines=3)]


def report(snap: dict, prev: dict | None) -> list[str]:
    langs = snap["languages"]
    rows = [[lang, *c.values()] for lang, c in langs.items()]
    rows.append(["total", *(sum(c[k] for c in langs.values()) for k in ("files", "code", "comment", "blank"))])
    out = table("lines per language (git ls-files, without vendor/)", ["", "files", "code", "comment", "blank"], rows)
    out += table("Rust code lines per area", ["", "code", "test"], [[n, *a.values()] for n, a in snap["areas"].items()])
    out += table("vendored Rust: all / compiled (default and gui, host)", ["", "files", "code", "lines", "files",
                 "code", "lines"], [[n, *v.values()] for n, v in snap["vendor"].items()])
    for key, title in (("cc", "cognitive complexity"), ("lines", "length (lines)")):
        rows = [[f"{f['name']}  {f['file']}:{f['line']}", f["cc"], f["lines"]] for f in top(snap["functions"], key)]
        out += table(f"top {TOP} by {title}, without test code", ["", "cc", "lines"], rows)
    return out + (trend(prev, snap) if prev else [])


def main() -> None:
    prev = json.loads(SNAPSHOT.read_text()) if SNAPSHOT.is_file() else None
    snap = collect()
    SNAPSHOT.write_text(json.dumps(snap, indent=1))
    print(json.dumps(snap, indent=1) if "--json" in sys.argv[1:] else "\n".join(report(snap, prev)))


if __name__ == "__main__":
    main()
