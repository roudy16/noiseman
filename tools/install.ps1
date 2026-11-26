# Install the latest noiseman release for this Windows machine, for the current user (no admin), with an `nsm` link:
#   irm https://github.com/roudy16/noiseman/releases/latest/download/install.ps1 | iex
# It installs the terminal build (CLI + TUI); with $env:NOISEMAN_GUI = '1' set first, the build with the desktop window.
# NOISEMAN_INSTALL_DIR overrides the directory (default %LOCALAPPDATA%\Programs\noiseman); NOISEMAN_VERSION (e.g.
# v0.1.0) picks a release. It adds the directory to the user's PATH, a Start menu shortcut (to the window, or the
# terminal UI in a console), and an entry in Settings > Apps whose Uninstall removes all of it. Windows counterpart of
# tools/install.sh.
# Everything is inside Install-Noiseman, called on the last line, so a download cut short runs nothing; it never calls
# `exit`, which would close the shell that ran `iex`.

function Install-Noiseman {
    $ErrorActionPreference = 'Stop'
    $ProgressPreference = 'SilentlyContinue'  # the progress bar slows downloads severalfold in Windows PowerShell
    $repo = 'https://github.com/roudy16/noiseman'
    $dir = if ($env:NOISEMAN_INSTALL_DIR) { $env:NOISEMAN_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA 'Programs\noiseman' }
    # Absolute, from PowerShell's current location (not .NET's, which GetFullPath uses), and it need not exist yet: the
    # PATH entry, shortcut and UninstallString must not depend on where the installer ran.
    $dir = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($dir)

    # AF_UNIX sockets, which the daemon listens on, arrived in Windows 10 1803 (build 17134).
    if ([Environment]::OSVersion.Version.Build -lt 17134) { throw 'noiseman needs Windows 10 version 1803 or later' }
    # The machine's own architecture, even from an emulated or 32-bit shell (Windows PowerShell 5.1 lacks
    # RuntimeInformation.OSArchitecture, and $env:PROCESSOR_ARCHITECTURE is the shell's).
    $arch = (Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Control\Session Manager\Environment').PROCESSOR_ARCHITECTURE
    $target = switch ($arch) {
        'AMD64' { 'x86_64-pc-windows-msvc' }
        'ARM64' { 'aarch64-pc-windows-msvc' }
        default { throw "no prebuilt binary for Windows on $arch; build from source ($repo)" }
    }
    $base = if ($env:NOISEMAN_VERSION) { "$repo/releases/download/$env:NOISEMAN_VERSION" } else { "$repo/releases/latest/download" }
    $gui = $env:NOISEMAN_GUI -eq '1'
    $name = if ($gui) { "noiseman-gui-$target" } else { "noiseman-$target" }

    $tmp = Join-Path ([IO.Path]::GetTempPath()) ("noiseman-" + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory $tmp | Out-Null
    try {
        [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
        Write-Host "Downloading $base/$name.zip"
        Invoke-WebRequest -UseBasicParsing "$base/$name.zip" -OutFile "$tmp\$name.zip"
        # Saved, not read from .Content: GitHub serves it as application/octet-stream, which makes .Content bytes.
        Invoke-WebRequest -UseBasicParsing "$base/$name.zip.sha256" -OutFile "$tmp\$name.zip.sha256"
        $want = ((Get-Content -Raw "$tmp\$name.zip.sha256") -split '\s+')[0]
        if ((Get-FileHash -Algorithm SHA256 "$tmp\$name.zip").Hash -ne $want) { throw 'checksum mismatch, not installing' }
        Expand-Archive "$tmp\$name.zip" $tmp
        $new = "$tmp\$name"
        & "$new\noiseman.exe" help | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "the $target binary does not run here; build from source ($repo)" }
        # A running player keeps running the replaced binary; stop it (`status` never starts one) so this version takes over.
        $stopped = $false
        $current = Join-Path $dir 'noiseman.exe'
        if (Test-Path $current) {
            $ErrorActionPreference = 'Continue'  # with 'Stop', Windows PowerShell throws on a native command's redirected stderr
            try {
                & $current status 2>$null | Out-Null
                if ($LASTEXITCODE -eq 0) {
                    & $current stop | Out-Null
                    if ($LASTEXITCODE -ne 0) { throw "``noiseman stop`` exited with $LASTEXITCODE" }
                    $stopped = $true
                }
            } catch {
                Write-Warning "cannot stop the running player ($_); run ``noiseman stop`` to restart it with this version"
            } finally {
                $ErrorActionPreference = 'Stop'
            }
        }

        New-Item -ItemType Directory -Force $dir | Out-Null
        # Leftovers of binaries that were running at the last install (see Move-Aside), and nothing else.
        Get-ChildItem -LiteralPath $dir -File | Where-Object { $_.Name -match $OldPattern } |
            Remove-Item -Force -ErrorAction SilentlyContinue
        # Only the gui build has noiseman-gui.exe; one from an earlier gui install goes, or the shortcut would run it.
        foreach ($exe in 'noiseman.exe', 'noiseman-gui.exe') {
            $path = Join-Path $dir $exe
            if (Test-Path $path) { Move-Aside $path }
            if (Test-Path "$new\$exe") { Copy-Item "$new\$exe" $path }
        }
        Copy-Item "$new\LICENSE", "$new\THIRD-PARTY-LICENSES.txt" $dir
        if ($gui) {
            Write-Host "Installed $dir\noiseman.exe, the gui build (CLI, TUI and desktop window)"
        } else {
            Write-Host "Installed $dir\noiseman.exe, the terminal build (CLI and TUI; for the desktop window, set"
            Write-Host "`$env:NOISEMAN_GUI = '1' and run the installer again)"
        }
        if ($stopped) { Write-Host 'Stopped the running player so this version takes over; `noiseman` starts it again' }
        Set-Nsm $dir
        $pathAdded = Add-UserPath $dir
        Add-Shortcut $dir
        Register-Uninstall $dir (Get-Item "$dir\noiseman.exe").VersionInfo.ProductVersion $pathAdded
    } finally {
        Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
    }
    Write-Host 'Open noiseman from the Start menu, or run `noiseman` in a new terminal.'
}

# The names Move-Aside gives; install and uninstall delete only these, so a shared NOISEMAN_INSTALL_DIR keeps its files.
$OldPattern = '^(noiseman|noiseman-gui|nsm)\.exe\.[0-9a-f]{32}\.old$'

# Renames a file that may be running (it cannot be overwritten then) and deletes it if it is not.
function Move-Aside($path) {
    $old = "$path.$([guid]::NewGuid().ToString('N')).old"
    Move-Item -Force $path $old
    Remove-Item -Force $old -ErrorAction SilentlyContinue
}

# Ours if its embedded product name is noiseman's (build.rs sets it).
function Test-Ours($path) { (Get-Item $path).VersionInfo.ProductName -eq 'noiseman' }

# `nsm.exe`, a hard link to noiseman.exe (a symlink would need Developer Mode or admin). Never replaces or shadows
# someone else's `nsm`: warns and leaves it be.
function Set-Nsm($dir) {
    $nsm = Join-Path $dir 'nsm.exe'
    $other = Get-Command nsm -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if ((Test-Path $nsm) -and -not (Test-Ours $nsm)) {
        Write-Warning "$nsm already exists and is not noiseman's; not linking nsm"
        Write-Warning "To type nsm for noiseman in PowerShell anyway, add this line to your profile (`$PROFILE): Set-Alias nsm noiseman"
    } elseif ($other -and $other.Source -ne $nsm -and -not (Test-Ours $other.Source)) {
        Write-Warning "nsm on PATH is $($other.Source), not noiseman; not linking nsm"
        Write-Warning "To type nsm for noiseman in PowerShell anyway, add this line to your profile (`$PROFILE): Set-Alias nsm noiseman"
    } else {
        # The old link still points at the previous binary (renamed aside): replace it the same way.
        if (Test-Path $nsm) { Move-Aside $nsm }
        New-Item -ItemType HardLink -Path $nsm -Target (Join-Path $dir 'noiseman.exe') | Out-Null
        Write-Host "Linked $nsm"
    }
}

# The user's PATH lives in HKCU\Environment, usually as REG_EXPAND_SZ with %VARIABLES% in it: edit it in place (the
# .NET SetEnvironmentVariable would rewrite it as REG_SZ and expand them), then tell running programs it changed.
# Returns whether it added the entry, so uninstall removes only an entry the installer made.
function Add-UserPath($dir) {
    $added = $false
    $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $true)
    try {
        $old = $key.GetValue('Path', '', 'DoNotExpandEnvironmentNames')
        if (($old -split ';') -notcontains $dir) {
            $sep = if ($old -and -not $old.EndsWith(';')) { ';' } else { '' }
            $key.SetValue('Path', "$old$sep$dir", [Microsoft.Win32.RegistryValueKind]::ExpandString)
            # Clearing an unset variable this way makes .NET tell running programs (Explorer) the environment changed.
            [Environment]::SetEnvironmentVariable('NOISEMAN_INSTALL', $null, 'User')
            Write-Host "Added $dir to your PATH (new terminals pick it up)"
            $added = $true
        }
    } finally { $key.Close() }
    if (($env:Path -split ';') -notcontains $dir) { $env:Path = "$env:Path;$dir" }
    $added
}

# The gui build's shortcut opens the window without a console; the terminal build's runs the TUI in a console.
function Add-Shortcut($dir) {
    $lnk = Join-Path ([Environment]::GetFolderPath('Programs')) 'noiseman.lnk'
    $s = (New-Object -ComObject WScript.Shell).CreateShortcut($lnk)
    $gui = Join-Path $dir 'noiseman-gui.exe'
    $s.TargetPath = if (Test-Path $gui) { $gui } else { Join-Path $dir 'noiseman.exe' }
    $s.WorkingDirectory = $dir
    $s.Description = 'Noise player'
    $s.Save()
    Write-Host "Added the Start menu shortcut $lnk"
}

# Settings > Apps lists the per-user entry; its Uninstall runs uninstall.ps1, written beside the binaries. It removes
# only what the installer wrote (the directory only once empty, the PATH entry only if the installer added it), and
# drops the entry last, so an uninstall blocked by a running copy can be run again.
$Uninstall = @'
# Removes noiseman as tools/install.ps1 installed it; Settings > Apps runs this.
$dir = $PSScriptRoot
$key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\noiseman'
& "$dir\noiseman.exe" stop | Out-Null
# The daemon fades out after replying, and an open window or TUI keeps its binary locked: wait, then end them. Only
# our own binaries (by name: a renamed-aside one may report either name), as the directory may be shared.
foreach ($p in @(Get-Process | Where-Object {
    $path = $_.Path
    $n = if ($path -and $path.StartsWith("$dir\", 'OrdinalIgnoreCase')) { $path.Substring($dir.Length + 1) }
    'noiseman.exe', 'noiseman-gui.exe' -contains $n -or $n -match '@OLD@' -or
        ($n -eq 'nsm.exe' -and $_.Product -eq 'noiseman')
})) {
    if (-not $p.WaitForExit(3000)) { $p.Kill(); [void]$p.WaitForExit(2000) }
}
$ours = 'noiseman.exe', 'noiseman-gui.exe', 'LICENSE', 'THIRD-PARTY-LICENSES.txt', 'uninstall.ps1'
$files = @(Get-ChildItem -LiteralPath $dir -File | Where-Object {
    $ours -contains $_.Name -or $_.Name -match '@OLD@' -or
    ($_.Name -eq 'nsm.exe' -and $_.VersionInfo.ProductName -eq 'noiseman')
})
$files | Remove-Item -Force -ErrorAction SilentlyContinue
$left = @($files | Where-Object { Test-Path -LiteralPath $_.FullName })
if ($left) {
    Write-Warning "cannot remove $($left.Name -join ', ') from $dir; close noiseman and run Uninstall again"
    Read-Host 'Press Enter to close'
    return
}
if (-not (Get-ChildItem -LiteralPath $dir -Force)) { Remove-Item -LiteralPath $dir }
Remove-Item -Force (Join-Path ([Environment]::GetFolderPath('Programs')) 'noiseman.lnk') -ErrorAction SilentlyContinue
if ((Get-ItemProperty $key -ErrorAction SilentlyContinue).PathAdded -eq 1) {
    $reg = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $true)
    $old = $reg.GetValue('Path', '', 'DoNotExpandEnvironmentNames')
    $reg.SetValue('Path', (($old -split ';' | Where-Object { $_ -ne $dir }) -join ';'), 'ExpandString')
    $reg.Close()
    [Environment]::SetEnvironmentVariable('NOISEMAN_INSTALL', $null, 'User')  # tells running programs PATH changed
}
# The lock file, log and saved state.
Remove-Item -Recurse -Force (Join-Path $env:LOCALAPPDATA 'noiseman') -ErrorAction SilentlyContinue
Remove-Item -Recurse -Force $key
'@

function Register-Uninstall($dir, $version, $pathAdded) {
    $script = Join-Path $dir 'uninstall.ps1'
    Set-Content -Encoding UTF8 -LiteralPath $script $Uninstall.Replace('@OLD@', $OldPattern)
    $key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\noiseman'
    # An upgrade finds the entry already on PATH: keep the first install's answer (New-Item -Force clears the key).
    $pathAdded = $pathAdded -or (Get-ItemProperty $key -ErrorAction SilentlyContinue).PathAdded -eq 1
    New-Item -Force $key | Out-Null
    New-ItemProperty -Force $key -Name PathAdded -Value ([int]$pathAdded) -PropertyType DWord | Out-Null
    $values = @{
        DisplayName = 'noiseman'; DisplayIcon = Join-Path $dir 'noiseman.exe'; InstallLocation = $dir
        Publisher = 'noiseman'; URLInfoAbout = 'https://github.com/roudy16/noiseman'
        UninstallString = "powershell.exe -NoProfile -ExecutionPolicy Bypass -File `"$script`""
    }
    if ($version) { $values.DisplayVersion = $version }
    foreach ($v in $values.GetEnumerator()) { New-ItemProperty -Force $key -Name $v.Key -Value $v.Value | Out-Null }
    foreach ($v in 'NoModify', 'NoRepair') { New-ItemProperty -Force $key -Name $v -Value 1 -PropertyType DWord | Out-Null }
}

Install-Noiseman
