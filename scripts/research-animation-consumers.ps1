param(
    [string]$GameDir = (Join-Path $PSScriptRoot '../..'),
    [string]$ToolPath = (Join-Path $env:TEMP 'jc2tool-material-water-research.exe')
)

# Read-only evidence probe. No Cargo, game launch, extraction or saved asset dumps.
$ErrorActionPreference = 'Stop'
$GameDir = (Resolve-Path -LiteralPath $GameDir).Path
if (-not (Test-Path -LiteralPath $ToolPath -PathType Leaf)) {
    throw 'Supply -ToolPath with an existing private prebuilt jc2tool executable.'
}
$bytes = [IO.File]::ReadAllBytes((Join-Path $GameDir 'JustCause2.exe'))
$pe = [BitConverter]::ToInt32($bytes, 0x3c)
if ([BitConverter]::ToUInt32($bytes, $pe) -ne 0x4550) { throw 'Invalid PE signature' }
$optional = $pe + 24
if ([BitConverter]::ToUInt16($bytes, $optional) -ne 0x10b) { throw 'Expected PE32' }
$imageBase = [BitConverter]::ToUInt32($bytes, $optional + 28)
$count = [BitConverter]::ToUInt16($bytes, $pe + 6)
$sections = $optional + [BitConverter]::ToUInt16($bytes, $pe + 20)
function Read-VaWord([uint32]$va) {
    $rva = $va - $imageBase
    for ($i = 0; $i -lt $count; $i++) {
        $s = $sections + 40 * $i
        $start = [BitConverter]::ToUInt32($bytes, $s + 12)
        $size = [BitConverter]::ToUInt32($bytes, $s + 16)
        if ($rva -ge $start -and $rva + 4 -le $start + $size) {
            $offset = [BitConverter]::ToUInt32($bytes, $s + 20) + $rva - $start
            return [BitConverter]::ToUInt32($bytes, [int]$offset)
        }
    }
    throw ('Unmapped VA {0:x8}' -f $va)
}
$hash = (Get-FileHash -LiteralPath (Join-Path $GameDir 'JustCause2.exe') -Algorithm SHA256).Hash
if ($hash -ne 'DF99A479354CBD46ED322BCBD6651938C39FC1E7432734586486DD9F53B74F80') {
    throw 'Executable hash differs from the researched retail version.'
}
Write-Output "Retail executable SHA256: $hash"
$checks = @(
    @(0x00eb72b8, 0x00817d10, 'CAnimationFinishCondition eval'),
    @(0x00eb73f0, 0x007340c0, 'CParachuteAnimationFinishCondition eval'),
    @(0x00f2453c, 0x009a4cf0, 'CAnimationState enter'),
    @(0x00f24544, 0x009a4cf0, 'CAnimationState exit'),
    @(0x00eb7294, 0x005145a0, 'CParachuteBlendState enter'),
    @(0x00eb729c, 0x005145a0, 'CParachuteBlendState exit'),
    @(0x00f24340, 0x420ff89e, 'event time key'),
    @(0x00f24344, 0x49f74dfd, 'Events key'),
    @(0x00f24348, [Convert]::ToUInt32('cb0cc3f4', 16), 'event selector key'),
    @(0x00f2434c, 0x065b3289, 'event argument key')
)
foreach ($check in $checks) {
    $actual = Read-VaWord $check[0]
    if ($actual -ne $check[1]) { throw "PE word mismatch: $($check[2])" }
    Write-Output ('{0:x8} -> {1:x8} {2}' -f $check[0], $actual, $check[2])
}
& $ToolPath hash Transition0 Transition1 Transition2 Transition3 Condition0 AND OR
if ($LASTEXITCODE -ne 0) { throw 'jc2tool hash failed' }
function Get-Value($node, [uint32]$key) {
    foreach ($entry in $node.values) { if ($entry[0] -eq $key) { return $entry[1] } }
}
function Show-SelectedStates($node) {
    $name = Get-Value $node 0x3a4ca003
    if ($name -in @('PCS_CLOSED', 'PCS_OPEN', 'PCS_OPENING_VERTICAL',
        'PCS_OPENING_HORIZONTAL', 'PCS_CLOSING', 'S_JUMP_START_STANDING',
        'S_JUMP_FLIGHT_STANDING')) {
        foreach ($child in $node.children) {
            if ($child[0] -eq 0x49e4a9fe) {
                # First four ordinals suffice for the documented bounded traces.
                foreach ($key in @(4005061344, 1804249278, 2875470898, 1383356566)) {
                    $ref = Get-Value $child[1] $key
                    if ($null -ne $ref) { Write-Output ('{0} {1:x8} -> {2}' -f $name, $key, $ref) }
                }
            }
        }
    }
    foreach ($child in $node.children) { Show-SelectedStates $child[1] }
}
foreach ($asset in @('parachute.afsm', 'basestatemachine.afsm')) {
    $json = & $ToolPath prop $GameDir $asset --json
    if ($LASTEXITCODE -ne 0) { throw "jc2tool failed reading $asset" }
    $graph = ($json -join "`n") | ConvertFrom-Json
    Write-Output "Fixture: $asset"
    foreach ($node in $graph.nodes) { Show-SelectedStates $node }
}
