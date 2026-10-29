# Runs one headless Cursor CLI session in the fixture for the enforcement investigation.
# The home directory is redirected to an isolated folder so user-level hooks, skills and
# agents (~/.cursor, ~/.claude) do not load; the CLI login still works (checked 2026-10-29).
param(
    [Parameter(Mandatory = $true)][string]$Test,
    [Parameter(Mandatory = $true)][string]$Prompt,
    [string]$Model = "cursor-grok-4.5-low",
    [switch]$NoTrust,
    [switch]$NoForce
)
$fixture = Split-Path -Parent $PSScriptRoot
$isolatedHome = "C:\temp\cursor-t3\home"
New-Item -ItemType Directory -Force $isolatedHome | Out-Null
$env:USERPROFILE = $isolatedHome
$env:HOME = $isolatedHome
Get-ChildItem env: | Where-Object { $_.Name -match '^CLAUDE' } | ForEach-Object { Remove-Item "env:$($_.Name)" }
Set-Content -NoNewline -Encoding ascii -Path (Join-Path $fixture "hooks\active-test.txt") -Value $Test
$cliArgs = @("-p", "--output-format", "json", "--model", $Model, "--workspace", $fixture)
if (-not $NoTrust) { $cliArgs += "--trust" }
if (-not $NoForce) { $cliArgs += "--force" }
$cliArgs += $Prompt
Set-Location $fixture
$out = & cursor-agent @cliArgs 2>&1
$code = $LASTEXITCODE
$text = ($out | Out-String)
$text | Set-Content -Encoding utf8 -Path (Join-Path $fixture "captures\$Test.result.json")
"EXIT $code"
$text
