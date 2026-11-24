#!/usr/bin/env python3
"""Dependency health: advisories for every locked crate, and drift of direct and vendored crates from crates.io.

Advisories come from OSV (https://osv.dev), which imports the RustSec database that `cargo audit` uses. Latest versions
and yanks come from the crates.io sparse index, the CDN cargo itself reads. Only public crate names and versions are
sent. Exit status 1 if a vulnerability affects a locked crate; drift, yanks and informational advisories (unmaintained,
unsound) are warnings.
"""

import json
import subprocess
import sys
import urllib.request
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
USER_AGENT = "noiseman-deps-check"
TIMEOUT = 20


def fetch(url: str, body: dict | None = None) -> bytes:
    data = json.dumps(body).encode() if body is not None else None
    headers = {"User-Agent": USER_AGENT, **({"Content-Type": "application/json"} if data else {})}
    with urllib.request.urlopen(urllib.request.Request(url, data, headers), timeout=TIMEOUT) as r:
        return r.read()


def version_key(v: str) -> tuple[int, ...]:
    return tuple(int(x) for x in v.split("+")[0].split("-")[0].split("."))


def compatible(a: str, b: str) -> bool:
    """Cargo's caret rule: same leftmost non-zero component."""
    ka, kb = version_key(a), version_key(b)
    i = next((i for i, x in enumerate(ka) if x != 0), len(ka) - 1)
    return ka[: i + 1] == kb[: i + 1]


def index_entries(name: str) -> list[dict]:
    n = name.lower()
    prefix = {1: "1", 2: "2", 3: f"3/{n[0]}"}.get(len(n), f"{n[:2]}/{n[2:4]}")
    return [json.loads(line) for line in fetch(f"https://index.crates.io/{prefix}/{n}").splitlines() if line]


def metadata() -> dict:
    out = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--all-features", "--locked"],
        cwd=ROOT, capture_output=True, text=True, check=True,
    ).stdout
    return json.loads(out)


def direct_and_vendored(meta: dict) -> dict[str, dict]:
    """name -> {version, vendored, req}: what we depend on by name, at the locked version."""
    packages = {p["id"]: p for p in meta["packages"]}
    members = set(meta["workspace_members"])
    picked: dict[str, dict] = {}
    for node in meta["resolve"]["nodes"]:
        if node["id"] not in members:
            continue
        reqs = {d["name"]: d["req"] for d in packages[node["id"]]["dependencies"]}
        for dep in node["deps"]:
            p = packages[dep["pkg"]]
            if p["id"] in members:
                continue
            vendored = "/vendor/" in p["manifest_path"]
            picked[p["name"]] = {"version": p["version"], "vendored": vendored, "req": reqs.get(p["name"], "?")}
    return picked


def drift(deps: dict[str, dict]) -> list[str]:
    def check(item: tuple[str, dict]) -> str | None:
        name, d = item
        entries = index_entries(name)
        stable = [e["vers"] for e in entries if not e["yanked"] and "-" not in e["vers"]]
        latest = max(stable, key=version_key)
        yanked = any(e["vers"] == d["version"] and e["yanked"] for e in entries)
        where = "vendored " if d["vendored"] else ""
        if yanked:
            return f"YANKED   {where}{name} {d['version']} (latest {latest})"
        if version_key(latest) <= version_key(d["version"]):
            return None
        if d["vendored"]:
            how = "re-vendor and re-apply or drop the patch (AGENTS.md)"
        elif compatible(d["version"], latest):
            how = "`cargo update` picks it up"
        else:
            how = f"needs a requirement change from {d['req']}"
        return f"OUTDATED {where}{name} {d['version']} -> {latest}: {how}"

    with ThreadPoolExecutor(8) as pool:
        return [line for line in pool.map(check, sorted(deps.items())) if line]


def advisories(meta: dict) -> tuple[list[str], list[str]]:
    crates = sorted({(p["name"], p["version"]) for p in meta["packages"] if p["id"] not in meta["workspace_members"]})
    queries = [{"package": {"name": n, "ecosystem": "crates.io"}, "version": v} for n, v in crates]
    results = json.loads(fetch("https://api.osv.dev/v1/querybatch", {"queries": queries}))["results"]
    hits = [(crate, vuln["id"]) for crate, r in zip(crates, results) for vuln in r.get("vulns", [])]

    def describe(hit: tuple[tuple[str, str], str]) -> tuple[bool, str]:
        (name, version), vuln_id = hit
        v = json.loads(fetch(f"https://api.osv.dev/v1/vulns/{vuln_id}"))
        # RustSec marks unmaintained/unsound/notice advisories on the affected entry, not the record.
        info = next(
            (a.get("database_specific", {}).get("informational") for a in v.get("affected", [])
             if a.get("package", {}).get("name") == name),
            None,
        )
        aliases = ", ".join(a for a in v.get("aliases", []) if a != vuln_id)
        label = f"{info.upper()}" if info else "VULN"
        text = f"{label:<8} {name} {version}: {vuln_id}{f' ({aliases})' if aliases else ''} {v.get('summary', '')}"
        return info is None, text.rstrip()

    with ThreadPoolExecutor(8) as pool:
        described = list(pool.map(describe, hits))
    return [t for real, t in described if real], [t for real, t in described if not real]


def main() -> int:
    meta = metadata()
    deps = direct_and_vendored(meta)
    try:
        drifted = drift(deps)
        vulns, notices = advisories(meta)
    except OSError as e:
        print(f"deps: cannot reach crates.io index or OSV: {e}", file=sys.stderr)
        return 2
    for line in vulns + notices + drifted:
        print(line)
    locked = len({(p["name"], p["version"]) for p in meta["packages"]}) - len(meta["workspace_members"])
    print(
        f"{len(vulns)} vulnerabilities, {len(notices)} advisories (informational), "
        f"{len(drifted)} of {len(deps)} direct/vendored crates behind; {locked} locked crates checked"
    )
    return 1 if vulns else 0


if __name__ == "__main__":
    sys.exit(main())
