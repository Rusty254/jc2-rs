[CmdletBinding(DefaultParameterSetName = 'Capture')]
param(
    [Parameter(Mandatory, ParameterSetName = 'Capture')][string]$Executable,
    [Parameter(Mandatory, ParameterSetName = 'Capture')][string]$GameDirectory,
    [Parameter(Mandatory, ParameterSetName = 'Capture')][string]$OutputDirectory,
    [Parameter(ParameterSetName = 'Capture')][string]$Model = 'vegetation_1.blz/Arctic_T22_PineL_LOD1-Whole.rbm',
    [Parameter(ParameterSetName = 'Capture')][ValidateRange(-360,360)][double]$Yaw = 215,
    [Parameter(ParameterSetName = 'Capture')][ValidateRange(-85,85)][double]$Pitch = 20,
    [Parameter(ParameterSetName = 'Capture')][ValidateRange(-360,360)][double]$LightYaw = 30,
    [Parameter(ParameterSetName = 'Capture')][ValidateRange(5,40)][int]$Seconds = 18,
    [Parameter(ParameterSetName = 'Capture')][switch]$DepthPrepass,
    [Parameter(ParameterSetName = 'Capture')][switch]$NoNormalMaps,
    [Parameter(Mandatory, ParameterSetName = 'Verify')][string]$CheckReceipt,
    [Parameter(ParameterSetName = 'Capture')]
    [Parameter(Mandatory, ParameterSetName = 'Verify')][string]$CompareReceipt
)
$ErrorActionPreference = 'Stop'

function Assert-CameraMatch($Candidate, $Reference) {
    foreach ($receipt in @($Candidate, $Reference)) {
        if ($receipt.exit -ne 0 -or !$receipt.scene.ready -or !$receipt.scene.locked) {
            throw 'A capture was not ready, locked or successful; camera comparison rejected.'
        }
    }
    # Lighting, normals and prepass settings may deliberately differ. Camera,
    # viewport and selected scene must agree before pixels can be compared.
    $candidateState = $Candidate.scene | Select-Object model,lod,hide_parts,camera | ConvertTo-Json -Depth 20 -Compress
    $referenceState = $Reference.scene | Select-Object model,lod,hide_parts,camera | ConvertTo-Json -Depth 20 -Compress
    if ($candidateState -cne $referenceState) {
        throw 'Final camera, viewport or scene differs; pixel comparison rejected. Inspect both receipts.'
    }
}

if ($PSCmdlet.ParameterSetName -eq 'Verify') {
    $candidate = Get-Content -LiteralPath $CheckReceipt -Raw | ConvertFrom-Json
    $reference = Get-Content -LiteralPath $CompareReceipt -Raw | ConvertFrom-Json
    Assert-CameraMatch $candidate $reference
    [ordered]@{ camera_matches=$true; candidate=$CheckReceipt; reference=$CompareReceipt } | ConvertTo-Json
    return
}

$binary = (Resolve-Path -LiteralPath $Executable).Path
$game = (Resolve-Path -LiteralPath $GameDirectory).Path
$output = [IO.Path]::GetFullPath($OutputDirectory)
[IO.Directory]::CreateDirectory($output) | Out-Null
$png = Join-Path $output 'capture.png'
$cameraPath = Join-Path $output 'capture.camera.json'
$receiptPath = Join-Path $output 'receipt.json'
$stdout = Join-Path $output 'capture.out'
$stderr = Join-Path $output 'capture.err'
foreach ($path in @($png, $cameraPath, $receiptPath, $stdout, $stderr)) {
    if (Test-Path -LiteralPath $path) { throw "Capture output exists; use a fresh directory: $path" }
}
$culture = [Globalization.CultureInfo]::InvariantCulture
$arguments = @($game, '--model', $Model, '--material-check', '--material-capture',
    '--camera-yaw', $Yaw.ToString($culture), '--camera-pitch', $Pitch.ToString($culture),
    '--light-yaw', $LightYaw.ToString($culture), '--exit-after', "$Seconds", '--screenshot', $png)
if ($DepthPrepass) { $arguments += '--depth-prepass' }
if ($NoNormalMaps) { $arguments += '--no-normal-maps' }
$quoted = $arguments | ForEach-Object { '"' + $_.Replace('"', '\"') + '"' }
$started = [DateTime]::UtcNow
$previousLog = $env:RUST_LOG
try {
    $env:RUST_LOG = 'info,jc2::rbm_loader=debug,wgpu_core=warn,wgpu_hal=warn,naga=warn'
    $process = Start-Process -FilePath $binary -ArgumentList ($quoted -join ' ') -WorkingDirectory $game -WindowStyle Hidden -PassThru -RedirectStandardOutput $stdout -RedirectStandardError $stderr
    $timer = [Diagnostics.Stopwatch]::StartNew()
    while (!$process.WaitForExit(500)) {
        if ($timer.Elapsed.TotalSeconds -gt 90) {
            $process.Kill() # Only this helper's own child.
            throw "Owned material capture timed out (PID $($process.Id))."
        }
    }
} finally {
    if ($null -eq $previousLog) { Remove-Item Env:RUST_LOG -ErrorAction SilentlyContinue }
    else { $env:RUST_LOG = $previousLog }
}
$combined = [IO.File]::ReadAllText($stdout) + [IO.File]::ReadAllText($stderr)
$scene = if (Test-Path -LiteralPath $cameraPath) { Get-Content -LiteralPath $cameraPath -Raw | ConvertFrom-Json } else { $null }
$receipt = [ordered]@{
    executable=$binary; sha256=(Get-FileHash -LiteralPath $binary -Algorithm SHA256).Hash
    arguments=$arguments; started_utc=$started.ToString('o'); exit=$process.ExitCode
    screenshot=$png; stdout=$stdout; stderr=$stderr; scene=$scene
}
$receipt | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath $receiptPath
if ($process.ExitCode -ne 0) { throw "Material capture exited $($process.ExitCode); inspect its logs." }
foreach ($path in @($png, $cameraPath)) {
    if (!(Test-Path -LiteralPath $path) -or (Get-Item -LiteralPath $path).LastWriteTimeUtc -lt $started) {
        throw "Material capture is absent or stale: $path"
    }
}
if (!$scene.ready -or !$scene.locked -or $combined -match '\bERROR\b|panicked at|error\[E\d+\]') {
    throw 'Material capture was not ready/locked or reported an error; inspect receipt and logs.'
}
if ($CompareReceipt) {
    $reference = Get-Content -LiteralPath $CompareReceipt -Raw | ConvertFrom-Json
    Assert-CameraMatch ([pscustomobject]$receipt) $reference
}
$receipt | ConvertTo-Json -Depth 20
