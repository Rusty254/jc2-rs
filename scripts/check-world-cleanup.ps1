param(
    [Parameter(Mandatory)][string]$Executable,
    [Parameter(Mandatory)][string]$GameDirectory,
    [Parameter(Mandatory)][string]$OutputDirectory,
    [ValidateRange(10,40)][int]$Seconds = 25
)
$ErrorActionPreference = 'Stop'
$binary = (Resolve-Path -LiteralPath $Executable).Path
$game = (Resolve-Path -LiteralPath $GameDirectory).Path
$output = [IO.Path]::GetFullPath($OutputDirectory)
[IO.Directory]::CreateDirectory($output) | Out-Null
$png = Join-Path $output 'world-31-14.png'
$stdout = Join-Path $output 'world-31-14.out'
$stderr = Join-Path $output 'world-31-14.err'
$arguments = @($game, '--world', '31,14', '--radius', '0', '--exit-after', "$Seconds", '--screenshot', $png)
$quoted = $arguments | ForEach-Object { '"' + $_.Replace('"', '\"') + '"' }
$started = [DateTime]::UtcNow
$previousLog = $env:RUST_LOG
try {
    $env:RUST_LOG = 'warn,jc2::world=info,jc2::smoke=info'
    $process = Start-Process -FilePath $binary -ArgumentList ($quoted -join ' ') -WorkingDirectory $game -WindowStyle Hidden -PassThru -RedirectStandardOutput $stdout -RedirectStandardError $stderr
    $timer = [Diagnostics.Stopwatch]::StartNew()
    while (!$process.WaitForExit(500)) {
        if ($timer.Elapsed.TotalSeconds -gt 75) {
            # Only this fixture's own child is terminated, never an existing game.
            $process.Kill()
            throw "Owned cleanup fixture timed out (PID $($process.Id))."
        }
    }
} finally {
    if ($null -eq $previousLog) { Remove-Item Env:RUST_LOG -ErrorAction SilentlyContinue }
    else { $env:RUST_LOG = $previousLog }
}
$combined = [IO.File]::ReadAllText($stdout) + [IO.File]::ReadAllText($stderr)
$staleCount = ([regex]::Matches($combined, '(?m)^.*(?:Entity despawned:|entity_command::despawn).*$')).Count
$receipt = [ordered]@{ executable=$binary; sha256=(Get-FileHash -LiteralPath $binary -Algorithm SHA256).Hash; exit=$process.ExitCode; seconds=$Seconds; stale_despawn_warnings=$staleCount; screenshot=$png; stdout=$stdout; stderr=$stderr }
$receipt | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $output 'receipt.json')
if ($process.ExitCode -ne 0 -or !(Test-Path -LiteralPath $png) -or (Get-Item -LiteralPath $png).LastWriteTimeUtc -lt $started) {throw 'Cleanup fixture failed or screenshot is absent/stale.'}
if ($combined -notmatch 'tile 31,14:' -or $combined -match 'panicked at|error\[E\d+\]') {throw 'Cleanup fixture did not finish world terrain registration cleanly.'}
if ($staleCount -ne 0) {throw "Cleanup fixture has $staleCount stale despawn warning matches; inspect both logs."}
$receipt | ConvertTo-Json