param(
    [string]$CobolImage = 'gnucobol-db2-build',
    [string]$RustImage = 'rust:1-alpine'
)

$ErrorActionPreference = 'Stop'
$workspace = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
Set-Location $workspace
$output = 'build/bank-demo'
New-Item -ItemType Directory -Force -Path $output | Out-Null

$programs = @(
    @{ Name = 'BNKMENU'; Map = 'BNK1MAI' },
    @{ Name = 'BNK1DAC'; Map = 'BNK1DAM' },
    @{ Name = 'BNK1TFN'; Map = 'BNK1TFM' }
)

foreach ($program in $programs) {
    $name = $program.Name
    $map = $program.Map
    cargo run --manifest-path extras/cobcics/Cargo.toml --bin cobcics-mapgen --locked -- `
        "reference/Bank-of-Z-main/src/base/cics/bms/$map.bms" "$output/$map.cpy"
    if ($LASTEXITCODE -ne 0) { throw "BMS map generation failed: $map" }
    & extras/preprocessors/target/debug/cics-preprocess.exe `
        "reference/Bank-of-Z-main/src/base/cics/cobol/$name.cbl" `
        "$output/$map.cpy" "$output/$name.cob"
    if ($LASTEXITCODE -ne 0) { throw "CICS conversion failed: $name" }
}

$compile = @'
set -eu
for program in BNKMENU BNK1DAC BNK1TFN; do
  build/cobc/cobc -x -I build/bank-demo -I extras/cobcics/copy \
    -I reference/Bank-of-Z-main/src/base/cics/copy \
    -o "build/bank-demo/$program" "build/bank-demo/$program.cob" \
    extras/cobcics/bridge.c
done
'@
docker run --rm -e LICENSE=accept -e COB_CONFIG_DIR=/src/config `
    -e COB_COPY_DIR=/src/copy -e COB_CFLAGS=-I/src `
    -e COB_LDFLAGS=-L/src/build/libcob/.libs `
    -e LD_LIBRARY_PATH=/src/build/libcob/.libs --entrypoint sh `
    -v "${workspace}:/src" -w /src $CobolImage -c $compile
if ($LASTEXITCODE -ne 0) { throw 'COBOL worker compilation failed' }

$buildRust = @'
set -eu
cargo build --locked --release --bin cobcicsd
cp /tmp/cobcics-target/release/cobcicsd /src/build/bank-demo/cobcicsd
'@
docker run --rm -v "${workspace}:/src" -w /src/extras/cobcics `
    -e CARGO_TARGET_DIR=/tmp/cobcics-target $RustImage sh -c $buildRust
if ($LASTEXITCODE -ne 0) { throw 'Rust host compilation failed' }

Write-Output "Built BNKMENU, BNK1DAC, BNK1TFN and cobcicsd in $output"
