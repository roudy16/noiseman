<#
Windows runtime gates: the daemon's memory, threads, CPU and wakeups playing and muted, TUI and CLI memory (the
counterparts of tools/gates.py's runtime gates, which read /proc and run on Linux only), plus the GUI at idle.
Limits: the [windows] section of gates.toml. `just gates-windows` builds the binaries and runs this.

  powershell -File tools/winperf.ps1 -Bin target/release/noiseman.exe [-GuiBin <gui build>] [-Runs N]

With -Runs above 1 it prints each measurement's range over the runs instead of gating: use it to re-baseline.
Threads are gated on the program's own threads; Windows adds thread-pool, COM and WASAPI threads of its own, which
are reported (`*_threads`) but not gated. Exit status 1 if any gate fails.
#>
param([string]$Bin, [string]$GuiBin, [int]$Runs = 1, [double]$Window = 2.0)
. "$PSScriptRoot\winperf-lib.ps1"

$Bin = (Resolve-Path $Bin).Path
if ($GuiBin) { $GuiBin = (Resolve-Path $GuiBin).Path }
$start = [Diagnostics.Stopwatch]::StartNew()

$results = @(1..$Runs | ForEach-Object {
    $r = Measure-Runtime $Bin $Window
    if ($GuiBin) { foreach ($kv in @((Measure-Gui $GuiBin 3.0).GetEnumerator())) { $r[$kv.Key] = $kv.Value } }
    , $r  # keep each run's dictionary whole
})

if ($Runs -gt 1) {
    foreach ($k in $results[0].Keys) {
        $v = $results | ForEach-Object { $_[$k] }
        $min = ($v | Measure-Object -Minimum).Minimum; $max = ($v | Measure-Object -Maximum).Maximum
        '{0,-24} {1,10:N2} .. {2,-10:N2} ({3})' -f $k, $min, $max, ($v -join ', ')
    }
    exit 0
}

# `key = number` lines of gates.toml's [windows] section (underscores are digit separators).
$limits = @{}; $section = ''
foreach ($line in Get-Content (Join-Path $PSScriptRoot '..\gates.toml')) {
    if ($line -match '^\s*\[(.+)\]') { $section = $Matches[1]; continue }
    if ($section -eq 'windows' -and $line -match '^\s*(\w+)\s*=\s*([\d_.]+)') { $limits[$Matches[1]] = [double]($Matches[2] -replace '_', '') }
}

$r = $results[0]; $failed = 0; $total = 0
$units = @{ kb = 'kB'; percent = '%'; per_s = '/s' }
foreach ($k in $r.Keys) {
    $unit = ($units.GetEnumerator() | Where-Object { $k.EndsWith($_.Key) } | Select-Object -First 1).Value
    if (-not $limits.ContainsKey($k)) { '     {0,-26} {1,12:N2} {2,-3} (not gated)' -f $k, $r[$k], $unit; continue }
    $total++; $ok = $r[$k] -le $limits[$k]; if (-not $ok) { $failed++ }
    '{0} {1,-26} {2,12:N2} {3,-3} (limit {4:N2} {3})' -f ($(if ($ok) { 'ok  ' } else { 'FAIL' })), $k, $r[$k], $unit, $limits[$k]
}
'{0}/{1} gates passed in {2:N2} s' -f ($total - $failed), $total, $start.Elapsed.TotalSeconds
if ($failed) { exit 1 }
