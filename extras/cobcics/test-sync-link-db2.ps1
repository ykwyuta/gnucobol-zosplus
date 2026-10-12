param([string]$CobolImage = 'gnucobol-db2-build')

$ErrorActionPreference = 'Stop'
$workspace = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
Set-Location $workspace
$output = 'build/cobcics-sync-test'
New-Item -ItemType Directory -Force -Path $output | Out-Null
if (-not (Test-Path extras/db2sql/.runtime.env)) {
    throw 'Create extras/db2sql/.runtime.env as described in extras/db2sql/README.md'
}

cargo run --manifest-path extras/cobcics/Cargo.toml --bin cobcics-mapgen --locked -- `
    reference/Bank-of-Z-main/src/base/cics/bms/BNK1MAI.bms "$output/BNK1MAI.cpy"
if ($LASTEXITCODE -ne 0) { throw 'BMS map generation failed' }
& extras/preprocessors/target/debug/cics-preprocess.exe extras/cobcics/sync-link-worker.cob `
    "$output/BNK1MAI.cpy" "$output/sync-link-worker.cob"
if ($LASTEXITCODE -ne 0) { throw 'Main CICS conversion failed' }
& extras/preprocessors/target/debug/cics-preprocess.exe extras/cobcics/sync-link-sub.cob `
    "$output/BNK1MAI.cpy" "$output/sync-link-sub.cics.cob"
if ($LASTEXITCODE -ne 0) { throw 'Linked CICS conversion failed' }
& extras/preprocessors/target/debug/db2sql-preprocess.exe -I reference/Bank-of-Z-main/src/base/cics/copy `
    "$output/sync-link-sub.cics.cob" "$output/sync-link-sub.cob"
if ($LASTEXITCODE -ne 0) { throw 'Linked SQL conversion failed' }

$driver = '/usr/local/lib64/python3.9/site-packages'
$compile = @'
set -eu
gcc -std=c11 -Wall -Wextra -Werror -fPIC -shared \
  -I/src -I/usr/local/lib64/python3.9/site-packages/clidriver/include \
  extras/db2sql/db2sql.c -L/src/build/libcob/.libs -lcob \
  -L/usr/local/lib64/python3.9/site-packages/clidriver/lib -ldb2 \
  -o build/cobcics-sync-test/libdb2sql.so
build/cobc/cobc -x -I build/cobcics-sync-test -I extras/cobcics/copy \
  -I copy -o build/cobcics-sync-test/sync-link-worker \
  build/cobcics-sync-test/sync-link-worker.cob \
  build/cobcics-sync-test/sync-link-sub.cob extras/cobcics/bridge.c
'@
docker run --rm -e LICENSE=accept -e COB_CONFIG_DIR=/src/config `
    -e COB_COPY_DIR=/src/copy -e COB_CFLAGS='-I/src -DCOBCICS_DB2SQL' `
    -e "COB_LDFLAGS=-L/src/build/libcob/.libs -L/src/$output -ldb2sql -Wl,-rpath-link,$driver/ibm_db.libs" `
    -e "LD_LIBRARY_PATH=/src/build/libcob/.libs:/src/${output}:$driver/clidriver/lib:$driver/ibm_db.libs" `
    --entrypoint sh -v "${workspace}:/src" -w /src $CobolImage -c $compile
if ($LASTEXITCODE -ne 0) { throw 'CICS/Db2 worker build failed' }

docker run --rm --env-file extras/db2sql/.runtime.env -e LICENSE=accept `
    -e COB_HOST_CODESET=UTF-8 `
    -e "LD_LIBRARY_PATH=/src/build/libcob/.libs:/src/${output}:$driver/clidriver/lib:$driver/ibm_db.libs" `
    --entrypoint sh -v "${workspace}:/src" -w /src $CobolImage -c `
    "python3 extras/cobcics/test-sync-link-db2.py $output/sync-link-worker"
if ($LASTEXITCODE -ne 0) { throw 'CICS/Db2 transaction test failed' }
