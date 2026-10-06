$ErrorActionPreference = 'Stop'

$edgePath = @(
    (Get-Command msedge.exe -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Source -ErrorAction SilentlyContinue),
    "${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe",
    "$env:ProgramFiles\Microsoft\Edge\Application\msedge.exe"
) | Where-Object { $_ -and (Test-Path -LiteralPath $_) } | Select-Object -First 1

if (-not $edgePath) {
    throw 'Microsoft Edge is required for the overlay DOM regression. Install Edge or run tests/browser/overlay-anchor.html in a Chromium browser.'
}

$testPage = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot 'overlay-anchor.html')).Path
$testUri = ([System.Uri]::new($testPage)).AbsoluteUri
$resultsDir = Join-Path (Split-Path -Parent $PSScriptRoot) '..\target\browser-test'
New-Item -ItemType Directory -Force -Path $resultsDir | Out-Null
$stdout = Join-Path $resultsDir 'overlay-anchor.stdout.html'
$stderr = Join-Path $resultsDir 'overlay-anchor.stderr.log'
$profile = Join-Path $resultsDir 'edge-profile'
$browser = Start-Process -FilePath $edgePath -Wait -PassThru `
    -ArgumentList @('--headless', '--disable-gpu', '--no-first-run', '--no-default-browser-check', '--allow-file-access-from-files', "--user-data-dir=$profile", '--dump-dom', $testUri) `
    -RedirectStandardOutput $stdout -RedirectStandardError $stderr
$output = Get-Content -LiteralPath $stdout -Raw -ErrorAction SilentlyContinue
if ($browser.ExitCode -ne 0 -or $output -notmatch 'data-result="pass"') {
    $errors = Get-Content -LiteralPath $stderr -Raw -ErrorAction SilentlyContinue
    throw "Overlay anchor DOM regression failed:`n$output`n$errors"
}

$result = [regex]::Match($output, '<pre id="test-result" data-result="pass">(?<result>.*?)</pre>')
Write-Output $result.Groups['result'].Value
