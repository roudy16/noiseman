#!/usr/bin/env python3
"""Print the license files of every third-party crate linked into noiseman (`--features gui`) for a target.

Release archives ship this as THIRD-PARTY-LICENSES.txt: MIT, BSD and Apache-2.0 all require their text to travel
with binary copies. Only normal dependencies reachable from the binary are listed, minus proc macros. A crate that
ships no license file gets the standard text of its license.
"""

import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PREFIXES = ("license", "licence", "copying", "notice", "copyright")
MIT = """MIT License (this crate ships no license file; the copyright holders are its listed authors)

Copyright (c) {holders}

Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated
documentation files (the "Software"), to deal in the Software without restriction, including without limitation the
rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit
persons to whom the Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or substantial portions of the
Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE
WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR
COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR
OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
"""


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: tools/licenses.py TARGET", file=sys.stderr)
        return 2
    meta = json.loads(subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--locked", "--features", "gui", "--filter-platform", sys.argv[1]],
        cwd=ROOT, capture_output=True, text=True, check=True,
    ).stdout)
    packages = {p["id"]: p for p in meta["packages"]}
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    members = set(meta["workspace_members"])
    root = next(i for i in members if packages[i]["name"] == "noiseman")

    seen, todo = {root}, [root]
    while todo:
        for dep in nodes[todo.pop()]["deps"]:
            normal = any(k["kind"] is None for k in dep["dep_kinds"])
            # Proc macros run in the compiler and are not linked in; neither are their own dependencies.
            macro = any("proc-macro" in t["kind"] for t in packages[dep["pkg"]]["targets"])
            if normal and not macro and dep["pkg"] not in seen:
                seen.add(dep["pkg"])
                todo.append(dep["pkg"])

    crates = sorted((packages[i] for i in seen - members), key=lambda p: (p["name"], p["version"]))
    print(f"noiseman bundles the following {len(crates)} crates. Their licenses follow.\n")
    apache = False
    for p in crates:
        print("=" * 100)
        print(f"{p['name']} {p['version']} ({p.get('license') or 'see files'})")
        print("=" * 100)
        files = sorted(f for f in Path(p["manifest_path"]).parent.iterdir()
                       if f.is_file() and f.name.lower().startswith(PREFIXES))
        if not files:
            license = p.get("license") or ""
            if "Apache-2.0" in license:
                apache = True
                print(f"This crate ships no license file. noiseman uses it under Apache-2.0 (declared: {license});")
                print("the full text of the Apache License 2.0 is at the end of this file.\n")
            elif "MIT" in license:
                print(MIT.format(holders=", ".join(p.get("authors") or [p["name"] + " authors"])))
            else:
                print(f"This crate ships no license file; its declared license is {license}.\n")
        for f in files:
            print(f"--- {f.name} ---\n{f.read_text(errors='replace').strip()}\n")
    if apache:
        print("=" * 100)
        print("Apache License 2.0 (for the crates above that ship no license file)")
        print("=" * 100)
        print((ROOT / "vendor/cpal/LICENSE").read_text().strip())
    return 0


if __name__ == "__main__":
    sys.exit(main())
