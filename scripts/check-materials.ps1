param(
    [Parameter(Mandatory)][string]$Executable,
    [Parameter(Mandatory)][string]$GameDirectory,
    [string]$OutputDirectory = 'target/material-check',
    [ValidateRange(12, 120)][int]$Seconds = 18,
    [switch]$IncludeHook
)
# Use a private executable copied while the shared build lock is held.
# Captures are ignored local evidence; no game archives are changed.
$ErrorActionPreference = 'Stop'
$binary = (Resolve-Path -LiteralPath $Executable).Path
$game = (Resolve-Path -LiteralPath $GameDirectory).Path
$output = [IO.Path]::GetFullPath($OutputDirectory)
[IO.Directory]::CreateDirectory($output) | Out-Null
$pine = 'climate/zone_1/vegetation_1.blz/Arctic_T22_PineL_LOD1-Whole.rbm'
$bus = 'lave.v046_buscoach.ee/v046-body_m_lod1.rbm'
$cases = @(
    @{Name='pine-main'; Model=$pine; Yaw=25; Pitch=35; Light=30; Prepass=$false; Flat=$false},
    @{Name='pine-depth'; Model=$pine; Yaw=25; Pitch=35; Light=30; Prepass=$true; Flat=$false},
    @{Name='pine-back-depth'; Model=$pine; Yaw=205; Pitch=35; Light=30; Prepass=$true; Flat=$false},
    @{Name='pine-light2-depth'; Model=$pine; Yaw=25; Pitch=35; Light=140; Prepass=$true; Flat=$false},
    @{Name='pine-flat-a'; Model=$pine; Yaw=25; Pitch=35; Light=30; Prepass=$true; Flat=$true},
    @{Name='pine-flat-b'; Model=$pine; Yaw=25; Pitch=35; Light=140; Prepass=$true; Flat=$true},
    @{Name='bus-normal-a'; Model=$bus; Yaw=120; Pitch=20; Light=30; Prepass=$true; Flat=$false},
    @{Name='bus-flat-a'; Model=$bus; Yaw=120; Pitch=20; Light=30; Prepass=$true; Flat=$true},
    @{Name='bus-normal-b'; Model=$bus; Yaw=120; Pitch=20; Light=140; Prepass=$true; Flat=$false},
    @{Name='bus-flat-b'; Model=$bus; Yaw=120; Pitch=20; Light=140; Prepass=$true; Flat=$true}
)
if ($IncludeHook) {
    $hook = 'general.blz/wea04_lod1-grapplinghook.rbm'
    $cases += @(
        @{Name='hook-linear-a'; Model=$hook; Yaw=25; Pitch=35; Light=30; Prepass=$true; Flat=$false},
        @{Name='hook-linear-b'; Model=$hook; Yaw=25; Pitch=35; Light=140; Prepass=$true; Flat=$false}
    )
}
foreach ($case in $cases) {
    $png = Join-Path $output ($case.Name + '.png')
    $arguments = @($game, '--model', $case.Model, '--material-check', '--camera-yaw',
        [string]$case.Yaw, '--camera-pitch', [string]$case.Pitch, '--light-yaw',
        [string]$case.Light, '--exit-after', [string]$Seconds, '--screenshot', $png)
    if ($case.Prepass) { $arguments += '--depth-prepass' }
    if ($case.Flat) { $arguments += '--no-normal-maps' }
    # Start-Process joins ArgumentList before CreateProcess; quote every argument.
    $quoted = $arguments | ForEach-Object { '"' + $_.Replace('"', '\"') + '"' }
    $startedAt = [DateTime]::UtcNow
    $process = Start-Process -FilePath $binary -ArgumentList ($quoted -join ' ') `
        -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $output ($case.Name + '.log')) `
        -RedirectStandardError (Join-Path $output ($case.Name + '.err'))
    $timer = [Diagnostics.Stopwatch]::StartNew()
    while (!$process.WaitForExit(500)) {
        if ($timer.Elapsed.TotalSeconds -gt ($Seconds + 45)) {
            $process.Kill()
            throw "Owned material fixture $($case.Name) timed out."
        }
    }
    if ($process.ExitCode -ne 0 -or !(Test-Path -LiteralPath $png)) {
        throw "Material fixture $($case.Name) failed; inspect its log and stderr."
    }
    $log = Get-Content -LiteralPath (Join-Path $output ($case.Name + '.log')) -Raw
    if ((Get-Item -LiteralPath $png).LastWriteTimeUtc -lt $startedAt -or
        $log -notmatch 'material-check model=' -or $log -notmatch 'material check: depth_prepass=') {
        throw "Material fixture $($case.Name) did not produce a fresh, loaded-model capture."
    }
    Write-Output "Captured $($case.Name): $png"
}
