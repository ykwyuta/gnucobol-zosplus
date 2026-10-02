param(
    [string]$CobolImage = 'gnucobol-db2-build',
    [string]$RustImage = 'rust:1-alpine'
)

$ErrorActionPreference = 'Stop'
$workspace = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
Set-Location $workspace
New-Item -ItemType Directory -Force -Path build/cobcics-japanese | Out-Null

docker run --rm -e COB_CONFIG_DIR=/src/config -e COB_COPY_DIR=/src/copy `
    -e COB_CFLAGS=-I/src -e COB_LDFLAGS=-L/src/build/libcob/.libs `
    --entrypoint build/cobc/cobc -v "${workspace}:/src" -w /src `
    $CobolImage -x -fdata-codeset=IBM-1399 -fsource-codeset=UTF-8 `
    -o build/cobcics-japanese/JPDEMO `
    extras/cobcics/japanese-worker.cob extras/cobcics/bridge.c
if ($LASTEXITCODE -ne 0) { throw 'Japanese COBOL worker build failed' }

docker run --rm -v "${workspace}:/src" -w /src/extras/cobcics `
    -e CARGO_TARGET_DIR=/tmp/cobcics-target $RustImage sh -c `
    'cargo build --offline --release --bin cobcicsd && cp /tmp/cobcics-target/release/cobcicsd /src/build/cobcics-japanese/cobcicsd'
if ($LASTEXITCODE -ne 0) { throw 'Japanese TN3270 host build failed' }

docker run --rm -e COB_HOST_CODESET=UTF-8 `
    -e LD_LIBRARY_PATH=/src/build/libcob/.libs `
    --entrypoint python3 -v "${workspace}:/src" -w /src `
    $CobolImage extras/cobcics/test-japanese-tn3270.py
if ($LASTEXITCODE -ne 0) { throw 'Japanese TN3270 round trip failed' }
