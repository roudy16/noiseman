<#
Windows runtime gates: the daemon's memory, threads, CPU and wakeups playing and muted, TUI and CLI memory (the
counterparts of tools/gates.py's runtime gates, which read /proc and run on Linux only). Limits: the [windows]
section of gates.toml. `just gates-windows` builds the binary and runs this:

  powershell -File tools/winperf.ps1 -Bin target/release/noiseman.exe

Threads are gated on the program's own; Windows adds thread-pool, COM and WASAPI threads of its own. Exit status 1 if
any gate fails.
#>
param([string]$Bin, [double]$Window = 2.0)
$ErrorActionPreference = 'Stop'

Add-Type @'
using System; using System.Net; using System.Net.Sockets; using System.Runtime.InteropServices; using System.Text;
public static class Mem {
    [StructLayout(LayoutKind.Sequential)]
    public struct PMC2 {
        public uint cb; public uint PageFaultCount; public UIntPtr PeakWorkingSetSize; public UIntPtr WorkingSetSize;
        public UIntPtr QuotaPeakPagedPoolUsage; public UIntPtr QuotaPagedPoolUsage; public UIntPtr QuotaPeakNonPagedPoolUsage;
        public UIntPtr QuotaNonPagedPoolUsage; public UIntPtr PagefileUsage; public UIntPtr PeakPagefileUsage;
        public UIntPtr PrivateUsage; public UIntPtr PrivateWorkingSetSize; public ulong SharedCommitUsage;
    }
    [DllImport("psapi.dll", SetLastError = true)]
    static extern bool GetProcessMemoryInfo(IntPtr h, out PMC2 c, uint cb);
    public static PMC2 Get(IntPtr h) {
        PMC2 c; if (!GetProcessMemoryInfo(h, out c, (uint)Marshal.SizeOf(typeof(PMC2)))) throw new System.ComponentModel.Win32Exception();
        return c;
    }
    [DllImport("kernel32.dll")] public static extern bool QueryProcessCycleTime(IntPtr h, out ulong cycles);
    [DllImport("kernel32.dll")] static extern IntPtr OpenThread(uint a, bool i, uint id);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr h);
    [DllImport("ntdll.dll")] static extern int NtQueryInformationThread(IntPtr h, int c, out IntPtr info, int len, IntPtr ret);
    [DllImport("psapi.dll", CharSet = CharSet.Unicode)] static extern uint GetMappedFileNameW(IntPtr proc, IntPtr addr, StringBuilder sb, uint n);
    // The module a thread started in (its Win32 start address), e.g. "noiseman.exe" or "ntdll.dll".
    public static string StartModule(IntPtr proc, uint tid) {
        IntPtr t = OpenThread(0x0040, false, tid); if (t == IntPtr.Zero) return "?";
        IntPtr addr; NtQueryInformationThread(t, 9, out addr, IntPtr.Size, IntPtr.Zero); CloseHandle(t);
        var sb = new StringBuilder(512); GetMappedFileNameW(proc, addr, sb, 512);
        string s = sb.ToString(); int k = s.LastIndexOf('\\'); return k >= 0 ? s.Substring(k + 1) : s;
    }
}
// The daemon's socket: .NET Framework sockets take AF_UNIX but have no endpoint type for it.
public class UnixEndPoint : EndPoint {
    string path; public UnixEndPoint(string p) { path = p; }
    public override AddressFamily AddressFamily { get { return AddressFamily.Unix; } }
    public override SocketAddress Serialize() {
        byte[] b = Encoding.UTF8.GetBytes(path); var a = new SocketAddress(AddressFamily.Unix, 2 + 108);
        for (int i = 0; i < b.Length; i++) a[2 + i] = b[i];
        return a;
    }
    public override EndPoint Create(SocketAddress a) { return this; }
}
'@

# The rate of the counter QueryProcessCycleTime reads: the processor's nominal clock.
$CycleHz = 1e6 * (Get-ItemProperty 'HKLM:\HARDWARE\DESCRIPTION\System\CentralProcessor\0').'~MHz'

function Kb($v) { [math]::Round([double]$v.ToUInt64() / 1024) }

# Private working set (resident pages only this process uses: the counterpart of Linux Private_Clean + Private_Dirty)
# and working set (RSS, which on Windows also counts shared system DLL pages).
function Memory($p) { $c = [Mem]::Get($p.Handle); @{ rss = Kb $c.WorkingSetSize; private = Kb $c.PrivateWorkingSetSize } }

# Context switches of every thread of the process, summed (raw, cumulative): its wakeups.
function Switches([int]$id) {
    (Get-CimInstance Win32_PerfRawData_PerfProc_Thread -Filter "IDProcess = $id" | Measure-Object ContextSwitchesPersec -Sum).Sum
}

# CPU % of one core and wakeups/s over `secs`. CPU comes from cycle counts: Windows charges process times in ~15.6 ms
# ticks, too coarse for a load of a few ms per second.
function Activity($p, [double]$secs) {
    $c0 = 0; [void][Mem]::QueryProcessCycleTime($p.Handle, [ref]$c0); $sw0 = Switches $p.Id; $t0 = [Diagnostics.Stopwatch]::StartNew()
    Start-Sleep -Milliseconds ([int]($secs * 1000))
    $c1 = 0; [void][Mem]::QueryProcessCycleTime($p.Handle, [ref]$c1); $sw1 = Switches $p.Id; $el = $t0.Elapsed.TotalSeconds
    @{ cpu = [math]::Round(100 * ($c1 - $c0) / ($CycleHz * $el), 3); wakeups = [math]::Round(($sw1 - $sw0) / $el, 1) }
}

# Threads the program started itself. The rest belong to Windows (thread pool, COM, WASAPI) and retire when idle.
function OwnThreads($p) {
    $p.Refresh(); $exe = $p.ProcessName + '.exe'
    @($p.Threads | Where-Object { [Mem]::StartModule($p.Handle, [uint32]$_.Id) -eq $exe }).Count
}

# The runtime gates' sequence (tools/gates.py): a private daemon started muted, then `vol 0` + `unmute` (a real
# WASAPI stream at zero gain: what playing costs, silently) with one client connected; a TUI; `noiseman status`; then
# muted with no clients.
$Bin = (Resolve-Path $Bin).Path
$start = [Diagnostics.Stopwatch]::StartNew()
$rundir = Join-Path $env:TEMP ('nm-gate-' + [guid]::NewGuid().ToString('N').Substring(0, 8))
New-Item -ItemType Directory $rundir | Out-Null
$env:XDG_RUNTIME_DIR = $rundir
$daemon = Start-Process $Bin -ArgumentList 'daemon', '--muted' -PassThru -WindowStyle Hidden -RedirectStandardError "$rundir\daemon.log"
$tui = $null; $conn = $null; $r = [ordered]@{}
try {
    $sock = New-Object Net.Sockets.Socket([Net.Sockets.AddressFamily]::Unix, [Net.Sockets.SocketType]::Stream, [Net.Sockets.ProtocolType]::Unspecified)
    $deadline = (Get-Date).AddSeconds(3)
    while ($true) {
        try { $sock.Connect((New-Object UnixEndPoint "$rundir\noiseman.sock")); break } catch {
            if ((Get-Date) -gt $deadline) { throw "the daemon did not start: $_" }
            Start-Sleep -Milliseconds 20
        }
    }
    $conn = New-Object Net.Sockets.NetworkStream($sock, $true)
    $rd = New-Object IO.StreamReader($conn); $wr = New-Object IO.StreamWriter($conn); $wr.AutoFlush = $true
    $wr.WriteLine('vol 0'); [void]$rd.ReadLine()
    $wr.WriteLine('unmute'); $state = $rd.ReadLine()
    if ($state -notmatch '^state \S+ 0 playing$') { throw "the daemon could not play: $state" }

    Start-Sleep -Milliseconds 300  # stream start-up
    $a = Activity $daemon $Window; $m = Memory $daemon
    $r.playing_private_kb = $m.private; $r.playing_rss_kb = $m.rss; $r.playing_own_threads = OwnThreads $daemon
    $r.playing_cpu_percent = $a.cpu; $r.playing_wakeups_per_s = $a.wakeups

    # The TUI in its own hidden console, on the same daemon.
    $tui = Start-Process $Bin -PassThru -WindowStyle Hidden
    Start-Sleep -Milliseconds 800  # connect and first draw
    $r.tui_private_kb = (Memory $tui).private
    Stop-Process -Id $tui.Id -Force; $tui.WaitForExit(); $tui = $null

    # `noiseman status`: peak working set, read from the exited process's still-open handle.
    $psi = New-Object Diagnostics.ProcessStartInfo($Bin, 'status')
    $psi.UseShellExecute = $false; $psi.RedirectStandardOutput = $true; $psi.CreateNoWindow = $true
    $cli = [Diagnostics.Process]::Start($psi); $null = $cli.StandardOutput.ReadToEnd(); $cli.WaitForExit()
    $r.cli_max_rss_kb = Kb ([Mem]::Get($cli.Handle)).PeakWorkingSetSize; $cli.Dispose()

    $wr.WriteLine('mute'); [void]$rd.ReadLine()
    $conn.Dispose(); $conn = $null  # the reader and writer wrap it; nothing left to flush
    $settle = (Get-Date).AddSeconds(1.5)
    while ((OwnThreads $daemon) -gt 2 -and (Get-Date) -lt $settle) { Start-Sleep -Milliseconds 10 }
    Start-Sleep -Milliseconds 300
    $a = Activity $daemon $Window
    $r.muted_private_kb = (Memory $daemon).private; $r.muted_own_threads = OwnThreads $daemon
    $r.muted_cpu_percent = $a.cpu; $r.muted_wakeups_per_s = $a.wakeups
} finally {
    if ($tui) { Stop-Process -Id $tui.Id -Force -ErrorAction SilentlyContinue }
    if ($conn) { $conn.Dispose() }
    Stop-Process -Id $daemon.Id -Force -ErrorAction SilentlyContinue; $daemon.WaitForExit()
    Remove-Item -Recurse -Force $rundir -ErrorAction SilentlyContinue
}

# `key = number` lines of gates.toml's [windows] section (underscores are digit separators).
$limits = @{}; $section = ''
foreach ($line in Get-Content (Join-Path $PSScriptRoot '..\gates.toml')) {
    if ($line -match '^\s*\[(.+)\]') { $section = $Matches[1]; continue }
    if ($section -eq 'windows' -and $line -match '^\s*(\w+)\s*=\s*([\d_.]+)') { $limits[$Matches[1]] = [double]($Matches[2] -replace '_', '') }
}

$failed = 0; $total = 0
$units = @{ kb = 'kB'; percent = '%'; per_s = '/s' }
foreach ($k in $r.Keys) {
    $unit = ($units.GetEnumerator() | Where-Object { $k.EndsWith($_.Key) } | Select-Object -First 1).Value
    if (-not $limits.ContainsKey($k)) { '     {0,-26} {1,12:N2} {2,-3} (not gated)' -f $k, $r[$k], $unit; continue }
    $total++; $ok = $r[$k] -le $limits[$k]; if (-not $ok) { $failed++ }
    '{0} {1,-26} {2,12:N2} {3,-3} (limit {4:N2} {3})' -f ($(if ($ok) { 'ok  ' } else { 'FAIL' })), $k, $r[$k], $unit, $limits[$k]
}
'{0}/{1} gates passed in {2:N2} s' -f ($total - $failed), $total, $start.Elapsed.TotalSeconds
if ($failed) { exit 1 }
