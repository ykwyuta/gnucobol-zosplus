param(
    [string]$CobolImage = 'gnucobol-db2-build',
    [string]$RustImage = 'rust:1-alpine'
)

$ErrorActionPreference = 'Stop'
$workspace = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
Set-Location $workspace
$output = 'build/bank-business'
New-Item -ItemType Directory -Force -Path "$output/workers" | Out-Null

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
    python bin/cics-preprocess.py `
        "reference/Bank-of-Z-main/src/base/cics/cobol/$name.cbl" `
        "$output/$map.cpy" "$output/$name.cob"
    if ($LASTEXITCODE -ne 0) { throw "CICS conversion failed: $name" }
}

foreach ($name in @('INQACC', 'DELACC', 'XFRFUN')) {
    python bin/cics-preprocess.py `
        "reference/Bank-of-Z-main/src/base/cics/cobol/$name.cbl" `
        "$output/BNK1DAM.cpy" "$output/workers/$name.cics.cob"
    if ($LASTEXITCODE -ne 0) { throw "CICS conversion failed: $name" }
    python bin/db2sql-preprocess.py -I reference/Bank-of-Z-main/src/base/cics/copy `
        "$output/workers/$name.cics.cob" "$output/workers/$name.gen.cob"
    if ($LASTEXITCODE -ne 0) { throw "SQL conversion failed: $name" }
}
python bin/cics-preprocess.py `
    reference/Bank-of-Z-main/src/base/cics/cobol/ABNDPROC.cbl `
    "$output/BNK1DAM.cpy" "$output/workers/ABNDPROC.gen.cob"
if ($LASTEXITCODE -ne 0) { throw 'CICS conversion failed: ABNDPROC' }

$compile = @'
set -eu
driver=/usr/local/lib64/python3.9/site-packages
gcc -std=c11 -Wall -Wextra -Werror -fPIC -shared \
  -I/src -I"$driver/clidriver/include" extras/db2sql/db2sql.c \
  -L/src/build/libcob/.libs -lcob -L"$driver/clidriver/lib" -ldb2 \
  -o build/bank-business/libdb2sql.so
includes='-I reference/Bank-of-Z-main/src/base/cics/copy -I extras/cobcics/copy -I build/bank-business'
build/cobc/cobc -x $includes -o build/bank-business/BNKMENU \
  build/bank-business/BNKMENU.cob extras/cobcics/bridge.c
build/cobc/cobc -x $includes -o build/bank-business/BNK1DAC \
  build/bank-business/BNK1DAC.cob \
  build/bank-business/workers/INQACC.gen.cob \
  build/bank-business/workers/DELACC.gen.cob \
  build/bank-business/workers/ABNDPROC.gen.cob extras/cobcics/bridge.c
build/cobc/cobc -x $includes -o build/bank-business/BNK1TFN \
  build/bank-business/BNK1TFN.cob \
  build/bank-business/workers/XFRFUN.gen.cob \
  build/bank-business/workers/ABNDPROC.gen.cob extras/cobcics/bridge.c
'@
$driver = '/usr/local/lib64/python3.9/site-packages'
docker run --rm -e LICENSE=accept -e COB_CONFIG_DIR=/src/config `
    -e COB_COPY_DIR=/src/copy -e COB_CFLAGS='-I/src -DCOBCICS_DB2SQL' `
    -e "COB_LDFLAGS=-L/src/build/libcob/.libs -L/src/$output -ldb2sql -Wl,-rpath-link,$driver/ibm_db.libs" `
    -e "LD_LIBRARY_PATH=/src/build/libcob/.libs:/src/${output}:$driver/clidriver/lib:$driver/ibm_db.libs" `
    --entrypoint sh -v "${workspace}:/src" -w /src $CobolImage -c $compile
if ($LASTEXITCODE -ne 0) { throw 'Bank business COBOL build failed' }

$buildRust = @'
set -eu
cargo build --locked --release --bin cobcicsd
cp /tmp/cobcics-target/release/cobcicsd /src/build/bank-business/cobcicsd
'@
docker run --rm -v "${workspace}:/src" -w /src/extras/cobcics `
    -e CARGO_TARGET_DIR=/tmp/cobcics-target $RustImage sh -c $buildRust
if ($LASTEXITCODE -ne 0) { throw 'Rust host compilation failed' }

Write-Output "Built BNKMENU, BNK1DAC + INQACC/DELACC, BNK1TFN + XFRFUN, ABNDPROC, and cobcicsd in $output"
