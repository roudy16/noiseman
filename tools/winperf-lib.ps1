# Measurement helpers for tools/winperf.ps1: the Windows counterparts of tools/gates.py's /proc readings.

$ErrorActionPreference = 'Stop'

Add-Type @'
using System; using System.Runtime.InteropServices;
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
    [DllImport("kernel32.dll")] static extern bool QueryThreadCycleTime(IntPtr h, out ulong cycles);
    [DllImport("kernel32.dll")] static extern IntPtr GetCurrentThread();
    [DllImport("kernel32.dll")] static extern IntPtr OpenThread(uint a, bool i, uint id);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr h);
    [DllImport("ntdll.dll")] static extern int NtQueryInformationThread(IntPtr h, int c, out IntPtr info, int len, IntPtr ret);
    [DllImport("psapi.dll", CharSet = CharSet.Unicode)] static extern uint GetMappedFileNameW(IntPtr proc, IntPtr addr, System.Text.StringBuilder sb, uint n);
    // The module a thread started in (its Win32 start address), e.g. "noiseman.exe" or "ntdll.dll".
    public static string StartModule(IntPtr proc, uint tid) {
        IntPtr t = OpenThread(0x0040, false, tid); if (t == IntPtr.Zero) return "?";
        IntPtr addr; NtQueryInformationThread(t, 9, out addr, IntPtr.Size, IntPtr.Zero); CloseHandle(t);
        var sb = new System.Text.StringBuilder(512); GetMappedFileNameW(proc, addr, sb, 512);
        string s = sb.ToString(); int k = s.LastIndexOf('\\'); return k >= 0 ? s.Substring(k + 1) : s;
    }
    // Cycles per second of the counter QueryProcessCycleTime reads, from 200 ms of busy work on this thread.
    public static double CycleHz() {
        ulong c0, c1; var sw = System.Diagnostics.Stopwatch.StartNew(); QueryThreadCycleTime(GetCurrentThread(), out c0);
        double x = 0; while (sw.ElapsedMilliseconds < 200) { x += 1; }
        QueryThreadCycleTime(GetCurrentThread(), out c1); return (c1 - c0) / sw.Elapsed.TotalSeconds;
    }
}
'@

$CycleHz = [Mem]::CycleHz()

function Kb($v) { [math]::Round([double]$v.ToUInt64() / 1024) }

# Private working set (resident pages only this process uses: the counterpart of Linux Private_Clean + Private_Dirty),
# working set (RSS, which on Windows also counts shared system DLL pages) and private commit.
function Memory($p) { $c = [Mem]::Get($p.Handle); @{ rss = Kb $c.WorkingSetSize; private = Kb $c.PrivateWorkingSetSize; commit = Kb $c.PrivateUsage } }

# Context switches of every thread of the process, summed (raw, cumulative): its wakeups.
function Switches([int]$id) {
    (Get-CimInstance Win32_PerfRawData_PerfProc_Thread -Filter "IDProcess = $id" | Measure-Object ContextSwitchesPersec -Sum).Sum
}

# CPU % of one core and wakeups/s over `secs`. CPU comes from cycle counts: Windows charges process times in ~15.6 ms
# ticks, too coarse for a load of a few ms per second.
function Activity($p, [double]$secs) {
    $p.Refresh(); $c0 = 0; [void][Mem]::QueryProcessCycleTime($p.Handle, [ref]$c0); $sw0 = Switches $p.Id; $t0 = [Diagnostics.Stopwatch]::StartNew()
    Start-Sleep -Milliseconds ([int]($secs * 1000))
    $p.Refresh(); $c1 = 0; [void][Mem]::QueryProcessCycleTime($p.Handle, [ref]$c1); $sw1 = Switches $p.Id; $el = $t0.Elapsed.TotalSeconds
    @{ cpu = [math]::Round(100 * ($c1 - $c0) / ($CycleHz * $el), 3); wakeups = [math]::Round(($sw1 - $sw0) / $el, 1) }
}

function Threads($p) { $p.Refresh(); $p.Threads.Count }

# Threads the program started itself. The rest belong to Windows (thread pool, COM, WASAPI) and retire when idle.
function OwnThreads($p) {
    $p.Refresh(); $exe = $p.ProcessName + '.exe'
    @($p.Threads | Where-Object { [Mem]::StartModule($p.Handle, [uint32]$_.Id) -eq $exe }).Count
}

function NewRunDir($prefix) {
    $dir = Join-Path $env:TEMP ($prefix + [guid]::NewGuid().ToString('N').Substring(0, 8))
    New-Item -ItemType Directory $dir | Out-Null
    $dir
}

# The runtime gates' sequence (tools/gates.py): a private daemon started muted, then `vol 0` + `unmute` (a real
# WASAPI stream at zero gain: what playing costs, silently) with one client connected; a TUI; `noiseman status`; then
# muted with no clients.
function Measure-Runtime([string]$Bin, [double]$Window) {
    $rundir = NewRunDir 'nm-gate-'
    $env:XDG_RUNTIME_DIR = $rundir
    $daemon = Start-Process $Bin -ArgumentList 'daemon', '--muted' -PassThru -WindowStyle Hidden -RedirectStandardError "$rundir\daemon.log"
    $tui = $null; $conn = $null; $r = [ordered]@{}
    try {
        $deadline = (Get-Date).AddSeconds(3)
        while (-not (Test-Path "$rundir\noiseman.pipe")) {
            if ((Get-Date) -gt $deadline) { throw 'the daemon did not publish its pipe' }
            Start-Sleep -Milliseconds 20
        }
        $short = ((Get-Content "$rundir\noiseman.pipe").Trim() -split '\\')[-1]
        $conn = New-Object System.IO.Pipes.NamedPipeClientStream('.', $short, [System.IO.Pipes.PipeDirection]::InOut)
        $conn.Connect(2000)
        $rd = New-Object System.IO.StreamReader($conn); $wr = New-Object System.IO.StreamWriter($conn); $wr.AutoFlush = $true
        $wr.WriteLine('vol 0'); [void]$rd.ReadLine()
        $wr.WriteLine('unmute'); $state = $rd.ReadLine()
        if ($state -notmatch '^state \S+ 0 playing$') { throw "the daemon could not play: $state" }

        Start-Sleep -Milliseconds 300  # stream start-up
        $a = Activity $daemon $Window; $m = Memory $daemon
        $r.playing_private_kb = $m.private; $r.playing_rss_kb = $m.rss
        $r.playing_own_threads = OwnThreads $daemon; $r.playing_threads = Threads $daemon
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
        $a = Activity $daemon $Window; $m = Memory $daemon
        $r.muted_private_kb = $m.private; $r.muted_own_threads = OwnThreads $daemon; $r.muted_threads = Threads $daemon
        $r.muted_cpu_percent = $a.cpu; $r.muted_wakeups_per_s = $a.wakeups
    } finally {
        if ($tui) { Stop-Process -Id $tui.Id -Force -ErrorAction SilentlyContinue }
        if ($conn) { $conn.Dispose() }
        Stop-Process -Id $daemon.Id -Force -ErrorAction SilentlyContinue; $daemon.WaitForExit()
        Remove-Item -Recurse -Force $rundir -ErrorAction SilentlyContinue
    }
    $r
}

# The GUI idle: the window attached to a muted private daemon (so nothing plays), after its first draw.
function Measure-Gui([string]$Bin, [double]$Window) {
    $rundir = NewRunDir 'nm-gui-'
    $env:XDG_RUNTIME_DIR = $rundir
    $daemon = Start-Process $Bin -ArgumentList 'daemon', '--muted' -PassThru -WindowStyle Hidden
    $gui = $null; $r = [ordered]@{}
    try {
        Start-Sleep -Milliseconds 500
        $gui = Start-Process $Bin -ArgumentList 'gui' -PassThru
        Start-Sleep -Milliseconds 1500  # window up, first draw
        $a = Activity $gui $Window; $m = Memory $gui
        $r.gui_private_kb = $m.private; $r.gui_rss_kb = $m.rss
        $r.gui_own_threads = OwnThreads $gui; $r.gui_threads = Threads $gui
        $r.gui_idle_cpu_percent = $a.cpu; $r.gui_idle_wakeups_per_s = $a.wakeups
    } finally {
        if ($gui) { Stop-Process -Id $gui.Id -Force -ErrorAction SilentlyContinue }
        & $Bin quit | Out-Null
        Stop-Process -Id $daemon.Id -Force -ErrorAction SilentlyContinue; $daemon.WaitForExit()
        Remove-Item -Recurse -Force $rundir -ErrorAction SilentlyContinue
    }
    $r
}
