param(
    [string]$Wc3270Path = 'C:\Program Files\wc3270\wc3270.exe',
    [string]$CobolImage = 'gnucobol-db2-build',
    [string]$RustImage = 'rust:1-alpine'
)

$ErrorActionPreference = 'Stop'
$workspace = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
Set-Location $workspace
if (-not (Test-Path -LiteralPath $Wc3270Path)) {
    throw "wc3270 not found: $Wc3270Path"
}
New-Item -ItemType Directory -Force -Path build/cobcics-japanese | Out-Null

docker run --rm -e COB_CONFIG_DIR=/src/config -e COB_COPY_DIR=/src/copy `
    -e COB_CFLAGS=-I/src -e COB_LDFLAGS=-L/src/build/libcob/.libs `
    --entrypoint build/cobc/cobc -v "${workspace}:/src" -w /src `
    $CobolImage -x -fdata-codeset=IBM-939 -fsource-codeset=UTF-8 `
    -o build/cobcics-japanese/JPDEMO939 `
    extras/cobcics/japanese-worker.cob extras/cobcics/bridge.c
if ($LASTEXITCODE -ne 0) { throw 'IBM-939 COBOL worker build failed' }

docker run --rm -v "${workspace}:/src" -w /src/extras/cobcics `
    -e CARGO_TARGET_DIR=/tmp/cobcics-target $RustImage sh -c `
    'cargo build --offline --release --bin cobcicsd && cp /tmp/cobcics-target/release/cobcicsd /src/build/cobcics-japanese/cobcicsd'
if ($LASTEXITCODE -ne 0) { throw 'Rust host build failed' }

function Get-FreePort {
    $listener = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback, 0)
    $listener.Start()
    $port = $listener.LocalEndpoint.Port
    $listener.Stop()
    return $port
}

$hostPort = Get-FreePort
$scriptPort = Get-FreePort
while ($scriptPort -eq $hostPort) { $scriptPort = Get-FreePort }
$container = "cobcics-japanese-wc3270-$PID"
$wcProcess = $null
$hostStarted = $false
$trace = Join-Path $workspace 'build/cobcics-japanese/wc3270-cp939-trace.txt'
try {
    docker run -d --rm --name $container -p "127.0.0.1:${hostPort}:3270" `
        -e COB_HOST_CODESET=UTF-8 -e LD_LIBRARY_PATH=/src/build/libcob/.libs `
        -v "${workspace}:/src" -w /src `
        --entrypoint /src/build/cobcics-japanese/cobcicsd $CobolImage `
        --bms /src/extras/cobcics/japanese-demo.bms --charset IBM-939 `
        --transid JP01 --route JP01=/src/build/cobcics-japanese/JPDEMO939 `
        --listen 0.0.0.0:3270 | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'Rust host startup failed' }
    $hostStarted = $true

    $wcArgs = @('+S', '-model', '3279-2', '-tn', 'IBM-3279-2',
                '-codepage', 'cp939', '-utf8', '-trace', '-tracefile', $trace,
                '-scriptport', "127.0.0.1:$scriptPort",
                "N:127.0.0.1:$hostPort")
    $wcProcess = Start-Process -FilePath $Wc3270Path -ArgumentList $wcArgs `
        -WindowStyle Hidden -PassThru
    python extras/cobcics/test-japanese-wc3270.py --script-port $scriptPort
    if ($LASTEXITCODE -ne 0) {
        docker logs --tail 30 $container
        if (Test-Path $trace) { Get-Content $trace -Tail 40 }
        throw 'wc3270 Japanese round trip failed'
    }
} finally {
    if ($wcProcess) { Stop-Process -Id $wcProcess.Id -Force -ErrorAction SilentlyContinue }
    if ($hostStarted) { docker stop $container | Out-Null }
}
