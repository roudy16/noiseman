#!/usr/bin/env python3
"""Quality gates: artifact sizes, DSP speed, daemon memory/threads/CPU/wakeups, TUI and CLI memory. Limits: gates.toml.

Expects release builds (`just gates` makes them). The runtime gates start a private daemon (own runtime dir) with
`--muted`, then `vol 0` + `unmute`: a real PulseAudio stream at zero gain, so it costs what playing costs, silently.
Without a PulseAudio server, or if the daemon cannot start a stream, they are reported as SKIP. They read /proc, so
they run on Linux only; elsewhere they are SKIP too, and the size and DSP gates still run. `--sizes-only` (CI, on
shared runners where timing and resource use mean little) runs only the size gates. Exit status 1 if any gate fails.
"""

import os
import re
import shutil
import socket
import struct
import subprocess
import sys
import tempfile
import time
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
EXE = ".exe" if sys.platform == "win32" else ""
BIN = ROOT / f"target/release/noiseman{EXE}"
WEB = ROOT / "target/web"
PROFILE = ROOT / f"target/release/examples/noise_profile{EXE}"

results: list[tuple[str, float, float, str]] = []


def gate(name: str, value: float, limit: float, unit: str) -> None:
    results.append((name, value, limit, unit))


def threads(pid: int) -> list[Path]:
    return list(Path(f"/proc/{pid}/task").iterdir())


def cpu_ns(pid: int) -> int:
    return sum(int(t.joinpath("schedstat").read_text().split()[0]) for t in threads(pid))


def switches(pid: int) -> int:
    total = 0
    for t in threads(pid):
        for line in t.joinpath("status").read_text().splitlines():
            if line.startswith(("voluntary_ctxt_switches", "nonvoluntary_ctxt_switches")):
                total += int(line.split()[1])
    return total


def memory_kb(pid: int) -> dict[str, int]:
    fields = {}
    for line in Path(f"/proc/{pid}/smaps_rollup").read_text().splitlines()[1:]:
        key, value = line.split(":")
        fields[key] = int(value.split()[0])
    return {"rss": fields["Rss"], "pss": fields["Pss"], "private": fields["Private_Clean"] + fields["Private_Dirty"]}


def activity(pid: int, secs: float) -> tuple[float, float]:
    """CPU percent and wakeups (context switches) per second over `secs`."""
    c0, s0, t0 = cpu_ns(pid), switches(pid), time.monotonic()
    time.sleep(secs)
    c1, s1, t1 = cpu_ns(pid), switches(pid), time.monotonic()
    return (c1 - c0) / 1e9 / (t1 - t0) * 100, (s1 - s0) / (t1 - t0)


def wait_until(check, timeout: float, what: str) -> None:
    end = time.monotonic() + timeout
    while not check():
        if time.monotonic() > end:
            raise RuntimeError(f"timed out waiting for {what}")
        time.sleep(0.01)


def sizes(limits: dict) -> None:
    gate("size: noiseman binary", BIN.stat().st_size, limits["noiseman_bytes"], "B")
    gate("size: noiseman.wasm", (WEB / "noiseman.wasm").stat().st_size, limits["wasm_bytes"], "B")
    total = sum(f.stat().st_size for f in WEB.iterdir() if f.is_file())
    gate("size: web site total", total, limits["web_total_bytes"], "B")


def dsp(limits: dict) -> None:
    out = subprocess.run([PROFILE, "all", "20"], capture_output=True, text=True, check=True).stdout
    for kind, ns in re.findall(r"^(\w+)\s.*?([\d.]+) ns/sample", out, re.M):
        gate(f"dsp: {kind.lower()} ns/sample", float(ns), limits["max_ns_per_sample"], "ns")


def runtime(limits: dict) -> None:
    if sys.platform != "linux":
        print(f"SKIP runtime gates: they measure through /proc (Linux only), not on {sys.platform}")
        return
    # Unix-only modules, imported here so the size and DSP gates run everywhere.
    import fcntl
    import pty
    import termios

    real_runtime = os.environ.get("XDG_RUNTIME_DIR", "")
    pulse = os.environ.get("PULSE_SERVER") or f"unix:{real_runtime}/pulse/native"
    if not Path(pulse.removeprefix("unix:")).is_socket():
        print(f"SKIP runtime gates: no PulseAudio server at {pulse}")
        return
    # A short path: the socket path must fit in sun_path (108 bytes).
    rundir = tempfile.mkdtemp(prefix="nm-gate-", dir="/tmp")
    env = {**os.environ, "XDG_RUNTIME_DIR": rundir, "PULSE_SERVER": pulse}
    env.pop("PULSE_RUNTIME_PATH", None)
    log = open(Path(rundir) / "daemon.log", "w")
    daemon = subprocess.Popen([BIN, "daemon", "--muted"], env=env, stdin=subprocess.DEVNULL, stderr=log)
    tui_pid = None
    try:
        sock_path = Path(rundir) / "noiseman.sock"
        wait_until(sock_path.exists, 2, "the daemon's socket")
        conn = socket.socket(socket.AF_UNIX)
        conn.connect(str(sock_path))
        replies = conn.makefile("r")

        def request(line: str) -> str:
            conn.sendall(f"{line}\n".encode())
            return replies.readline().strip()

        request("vol 0")
        state = request("unmute")
        if not state.startswith("state ") or state.split()[3] != "playing" or len(state.split()) > 4:
            print(f"SKIP runtime gates: the daemon could not play ({state})")
            return

        # The daemon alone first: while a TUI runs, the binary's code pages count as shared, not private.
        time.sleep(0.3)  # stream start-up
        cpu, wakeups = activity(daemon.pid, 0.4)
        mem = memory_kb(daemon.pid)
        gate("daemon playing: private memory", mem["private"], limits["playing_private_kb"], "kB")
        gate("daemon playing: RSS", mem["rss"], limits["playing_rss_kb"], "kB")
        gate("daemon playing: threads", len(threads(daemon.pid)), limits["playing_threads"], "")
        gate("daemon playing: CPU", cpu, limits["playing_cpu_percent"], "%")
        gate("daemon playing: wakeups", wakeups, limits["playing_wakeups_per_s"], "/s")

        tui_pid, tui_fd = pty.fork()
        if tui_pid == 0:
            os.execve(BIN, [str(BIN)], env)
        fcntl.ioctl(tui_fd, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 80, 0, 0))
        time.sleep(0.25)  # connect and first draw
        tui = memory_kb(tui_pid)
        gate("tui: private memory", tui["private"], limits["tui_private_kb"], "kB")
        os.write(tui_fd, b"q")
        os.waitpid(tui_pid, 0)
        os.close(tui_fd)
        tui_pid = None

        # Through GNU time: a child forked from Python would inherit Python's RSS as its high-water mark.
        if Path("/usr/bin/time").exists():
            peak = Path(rundir) / "cli-peak"
            cmd = ["/usr/bin/time", "-f", "%M", "-o", peak, BIN, "status"]
            subprocess.run(cmd, env=env, stdout=subprocess.DEVNULL, check=True)
            gate("cli status: peak RSS", int(peak.read_text()), limits["cli_max_rss_kb"], "kB")
        else:
            print("SKIP cli gate: /usr/bin/time (GNU time) not installed")

        request("mute")
        replies.close()  # makefile() holds the socket open too
        conn.close()
        # Muted with no clients is 2 threads; a regression shows up as a failed gate below, not a timeout.
        settle_end = time.monotonic() + 1.5
        while len(threads(daemon.pid)) > limits["muted_threads"] and time.monotonic() < settle_end:
            time.sleep(0.01)
        _, wakeups = activity(daemon.pid, 0.3)
        mem = memory_kb(daemon.pid)
        gate("daemon muted: private memory", mem["private"], limits["muted_private_kb"], "kB")
        gate("daemon muted: threads", len(threads(daemon.pid)), limits["muted_threads"], "")
        gate("daemon muted: wakeups", wakeups, limits["muted_wakeups_per_s"], "/s")
    finally:
        if tui_pid:
            os.kill(tui_pid, 9)
            os.waitpid(tui_pid, 0)
        daemon.kill()
        daemon.wait()
        log.close()
        shutil.rmtree(rundir, ignore_errors=True)


def main() -> int:
    sizes_only = "--sizes-only" in sys.argv[1:]
    limits = tomllib.loads((ROOT / "gates.toml").read_text())
    for path in (BIN, WEB / "noiseman.wasm") + (() if sizes_only else (PROFILE,)):
        if not path.exists():
            print(f"gates: missing {path.relative_to(ROOT)}; run `just gates`", file=sys.stderr)
            return 1
    start = time.monotonic()
    sizes(limits["size"])
    if sizes_only:
        print("SKIP dsp and runtime gates: timing and resource gates are for local runs (`just gates`)")
    else:
        dsp(limits["dsp"])
        runtime(limits["runtime"])
    failed = 0
    for name, value, limit, unit in results:
        ok = value <= limit
        failed += not ok
        shown = f"{value:,.2f}" if isinstance(value, float) else f"{value:,}"
        print(f"{'ok  ' if ok else 'FAIL'} {name:<34} {shown:>12} {unit:<3} (limit {limit:,} {unit})")
    print(f"{len(results) - failed}/{len(results)} gates passed in {time.monotonic() - start:.2f} s")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
