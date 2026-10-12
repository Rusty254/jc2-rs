<#
Runs the parachute capture scenarios (JC2-950d, research/formats/parachute-parity.md section 5)
one after another, each under the shared build lock (one game window at a time, no build
running beside it), and writes the frames to _extract/parachute (or -OutputDirectory).

  scripts/chute-capture.ps1                          all six scenarios
  scripts/chute-capture.ps1 -Scenario bank-hard      one
  scripts/chute-capture.ps1 -Executable C:\path\jc2.exe

Output per scenario: <scenario>_<instant>.png, <scenario>.md (what to compare with which
reference shots) and <scenario>.json (game time, Rico's state and the camera per frame);
index.md joins the tables. The runtime itself is `jc2 <game> --chute-scenario <name>`.
Frames are the window's size (1280x720 by default), not the reference shots' 2561x1429.
#>
[CmdletBinding()]
param(
    [string[]]$Scenario = @('jungle-deploy-day', 'base-night-behind', 'bank-hard',
                            'landing-collapse', 'canopy-from-above', 'freefall-poses'),
    [string]$Executable,
    [string]$GameDirectory,
    [string]$OutputDirectory,
    [string]$Actor = 'chute-capture',
    [int]$TimeoutSeconds = 420
)
$ErrorActionPreference = 'Stop'
# `-File` passes a comma list as one string
$Scenario = @($Scenario | ForEach-Object { $_ -split ',' } | Where-Object { $_ })
$repo = Split-Path -Parent $PSScriptRoot
if (-not $GameDirectory) {
    # the game folder is the one with archives_win32 above the repository (or a worktree of it)
    $dir = $repo
    while ($dir -and -not (Test-Path -LiteralPath (Join-Path $dir 'archives_win32'))) {
        $dir = Split-Path -Parent $dir
    }
    if (-not $dir) { throw 'Game folder not found; pass -GameDirectory.' }
    $GameDirectory = $dir
}
$game = (Resolve-Path -LiteralPath $GameDirectory).Path
if (-not $Executable) { $Executable = Join-Path $repo 'target/release/jc2.exe' }
$exe = (Resolve-Path -LiteralPath $Executable).Path
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $game 'jc2-rs/_extract/parachute' }
$out = [IO.Path]::GetFullPath($OutputDirectory)
[IO.Directory]::CreateDirectory($out) | Out-Null
$lock = Join-Path $PSScriptRoot 'with-build-lock.sh'
$failed = @()
# Git's bash (a `bash` on the path may be WSL's, which cannot run the lock script by Windows path)
$bash = $null
$gitBin = Split-Path -Parent (Split-Path -Parent (Get-Command git).Source)
foreach ($c in @((Join-Path $gitBin 'bin\bash.exe'), 'C:\Program Files\Git\bin\bash.exe')) {
    if (Test-Path -LiteralPath $c) { $bash = $c; break }
}
if (-not $bash) { throw 'Git bash not found.' }

foreach ($name in $Scenario) {
    Write-Host "== $name"
    $log = Join-Path $out "$name.log"
    # bash wants forward slashes
    $f = { param($path) $path.Replace([string][char]92, '/') }
    $cmd = "`"$(& $f $lock)`" $Actor `"$(& $f $exe)`" `"$(& $f $game)`" --chute-scenario $name --chute-out `"$(& $f $out)`""
    $argLine = @('-c', "'$cmd'")
    $p = Start-Process -FilePath $bash -ArgumentList $argLine -WorkingDirectory $game `
        -WindowStyle Hidden -PassThru -RedirectStandardOutput $log -RedirectStandardError "$log.err"
    $null = $p.Handle # keeps the exit code readable
    if (-not $p.WaitForExit($TimeoutSeconds * 1000)) {
        # only this helper's own child (and the game under it)
        Get-CimInstance Win32_Process -Filter "ParentProcessId=$($p.Id)" | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
        $p.Kill()
        Write-Warning "$name timed out after $TimeoutSeconds s"
        $failed += $name
        continue
    }
    $p.WaitForExit()
    $receipt = Join-Path $out "$name.json"
    $ok = (Test-Path -LiteralPath $receipt) -and ((Get-Content -LiteralPath $receipt -Raw | ConvertFrom-Json).complete)
    if ($p.ExitCode -ne 0 -or -not $ok) { Write-Warning "$name did not finish (exit $($p.ExitCode))"; $failed += $name }
}

$index = @('# Parachute capture frames', '', "Written by scripts/chute-capture.ps1 ($(Get-Date -Format s)). Positions and camera angles are non-native (the reference positions are not recorded).", '')
foreach ($name in $Scenario) {
    $table = Join-Path $out "$name.md"
    if (Test-Path -LiteralPath $table) { $index += (Get-Content -LiteralPath $table); $index += '' }
}
Set-Content -LiteralPath (Join-Path $out 'index.md') -Value $index -Encoding utf8
if ($failed.Count) { throw "Scenarios failed: $($failed -join ', ')" }
Write-Host "Frames in $out"
