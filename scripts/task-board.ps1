# Compatibility entry point: the JSON registry has been retired.
param(
    [string]$Action = 'list',
    [string]$Id,
    [Parameter(ValueFromRemainingArguments = $true)]$RemainingArguments
)
$ErrorActionPreference = 'Stop'
$tasks = Invoke-RestMethod 'http://127.0.0.1:4310/api/tasks'
$projectTasks = @($tasks | Where-Object { $_.tags -contains 'jc2-rs' })
if ($Action -eq 'list') {
    $projectTasks | Select-Object id, title, status, assignee, version | Format-Table -AutoSize -Wrap
} elseif ($Action -eq 'show') {
    $projectTasks | Where-Object { $_.id -eq $Id -or $_.tags -contains $Id } | ConvertTo-Json -Depth 15
} else {
    throw 'The file board is retired. Use the Relay API at http://127.0.0.1:4310/api. See TASKS.md for claim, PATCH and comments.'
}
