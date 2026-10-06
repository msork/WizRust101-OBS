$ErrorActionPreference = 'Stop'

$edgePath = @(
    (Get-Command msedge.exe -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Source -ErrorAction SilentlyContinue),
    "${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe",
    "$env:ProgramFiles\Microsoft\Edge\Application\msedge.exe"
) | Where-Object { $_ -and (Test-Path -LiteralPath $_) } | Select-Object -First 1
if (-not $edgePath) { throw 'Microsoft Edge is required for the live overlay regression.' }

$repoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\..')).Path
$resultsDir = Join-Path $repoRoot 'target\browser-test'
$examplePath = Join-Path $repoRoot 'target\debug\examples\overlay-live-fixture.exe'
New-Item -ItemType Directory -Force -Path $resultsDir | Out-Null
& cargo build --manifest-path (Join-Path $repoRoot 'Cargo.toml') --example overlay-live-fixture
if ($LASTEXITCODE -ne 0) { throw 'Could not build the live overlay fixture.' }

$profile = Join-Path $resultsDir 'edge-live-profile'
$fixture = Start-Process -FilePath $examplePath -PassThru -WindowStyle Hidden `
    -RedirectStandardOutput (Join-Path $resultsDir 'fixture.stdout.log') `
    -RedirectStandardError (Join-Path $resultsDir 'fixture.stderr.log')
$browser = $null
try {
    $ready = $false
    for ($attempt = 0; $attempt -lt 100; $attempt++) {
        if ($fixture.HasExited) { throw 'Live fixture exited before accepting connections.' }
        try {
            Invoke-WebRequest -Uri 'http://127.0.0.1:17849/__test/live-overlay-runner' -TimeoutSec 1 | Out-Null
            $ready = $true
            break
        } catch { Start-Sleep -Milliseconds 100 }
    }
    if (-not $ready) { throw 'Live fixture did not start at 127.0.0.1:17849.' }

    $browser = Start-Process -FilePath $edgePath -PassThru -WindowStyle Hidden `
        -ArgumentList @('--headless', '--disable-gpu', '--no-first-run', '--no-default-browser-check', '--remote-allow-origins=*', '--remote-debugging-port=17850', "--user-data-dir=$profile")
    $versionUri = 'http://127.0.0.1:17850/json/version'
    $debugReady = $false
    for ($attempt = 0; $attempt -lt 100; $attempt++) {
        if ($browser.HasExited) { throw 'Headless Edge exited before enabling its test control port.' }
        try {
            Invoke-RestMethod -Uri $versionUri -TimeoutSec 1 | Out-Null
            $debugReady = $true
            break
        } catch { Start-Sleep -Milliseconds 100 }
    }
    if (-not $debugReady) { throw 'Headless Edge did not expose its local test control port.' }

    $pageUri = [uri]::EscapeDataString('http://127.0.0.1:17849/__test/live-overlay-runner')
    $target = Invoke-RestMethod -Method Put -Uri "http://127.0.0.1:17850/json/new?$pageUri" -TimeoutSec 5
    $socket = [System.Net.WebSockets.ClientWebSocket]::new()
    $socket.Options.SetRequestHeader('Origin', 'http://localhost')
    [void]$socket.ConnectAsync([uri]$target.webSocketDebuggerUrl, [Threading.CancellationToken]::None).GetAwaiter().GetResult()
    $sequence = 0
    $deadline = [DateTime]::UtcNow.AddSeconds(45)
    $result = $null
    while ([DateTime]::UtcNow -lt $deadline) {
        $sequence++
        $expression = "(() => { const element = document.querySelector('#test-result'); return element ? JSON.stringify({ result: element.dataset.result, text: element.textContent }) : 'page-not-ready'; })()"
        $command = @{ id = $sequence; method = 'Runtime.evaluate'; params = @{ expression = $expression; returnByValue = $true } } | ConvertTo-Json -Depth 8 -Compress
        $bytes = [Text.Encoding]::UTF8.GetBytes($command)
        [void]$socket.SendAsync([ArraySegment[byte]]::new($bytes), [System.Net.WebSockets.WebSocketMessageType]::Text, $true, [Threading.CancellationToken]::None).GetAwaiter().GetResult()
        do {
            $buffer = [byte[]]::new(16384)
            $stream = [IO.MemoryStream]::new()
            do {
                $received = $socket.ReceiveAsync([ArraySegment[byte]]::new($buffer), [Threading.CancellationToken]::None).GetAwaiter().GetResult()
                if ($received.MessageType -eq [System.Net.WebSockets.WebSocketMessageType]::Close) { throw 'Edge closed the browser control socket.' }
                $stream.Write($buffer, 0, $received.Count)
            } while (-not $received.EndOfMessage)
            $message = [Text.Encoding]::UTF8.GetString($stream.ToArray()) | ConvertFrom-Json -AsHashtable
            $stream.Dispose()
        } while ($message.id -ne $sequence)
        $value = $message.result.result.value
        if ($value -and $value -ne 'page-not-ready') {
            $result = $value | ConvertFrom-Json -AsHashtable
            if ($result.result -in @('pass', 'fail')) { break }
        }
        Start-Sleep -Milliseconds 150
    }
    if (-not $result) { throw 'Browser regression did not finish within 45 seconds.' }
    if ($result.result -ne 'pass') { throw "Live overlay SSE regression failed:`n$($result.text)" }
    Write-Output $result.text
} finally {
    if ($socket -and $socket.State -eq [System.Net.WebSockets.WebSocketState]::Open) {
        [void]$socket.CloseAsync([System.Net.WebSockets.WebSocketCloseStatus]::NormalClosure, 'test complete', [Threading.CancellationToken]::None).GetAwaiter().GetResult()
    }
    if ($target) { Invoke-RestMethod -Method Put -Uri "http://127.0.0.1:17850/json/close/$($target.id)" -TimeoutSec 2 -ErrorAction SilentlyContinue | Out-Null }
    if ($browser -and -not $browser.HasExited) { Stop-Process -Id $browser.Id -Force }
    if (-not $fixture.HasExited) { Stop-Process -Id $fixture.Id -Force }
}
