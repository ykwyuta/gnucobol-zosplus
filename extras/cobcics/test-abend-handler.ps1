param([string]$CobolImage = 'gnucobol-db2-build')

$ErrorActionPreference = 'Stop'
$workspace = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
Set-Location $workspace
$output = 'build/bank-business'
if (-not (Test-Path "$output/workers/ABNDPROC.gen.cob")) {
    throw 'Run extras/cobcics/build-bank-business.ps1 first'
}
python bin/cics-preprocess.py extras/cobcics/abend-handler-worker.cob `
    "$output/BNK1DAM.cpy" "$output/workers/abend-handler-worker.gen.cob"
if ($LASTEXITCODE -ne 0) { throw 'ABEND handler conversion failed' }

$compile = @'
build/cobc/cobc -x \
  -I reference/Bank-of-Z-main/src/base/cics/copy \
  -I extras/cobcics/copy -I build/bank-business \
  -o build/bank-business/workers/abend-handler-worker \
  build/bank-business/workers/abend-handler-worker.gen.cob \
  build/bank-business/workers/ABNDPROC.gen.cob extras/cobcics/bridge.c
'@
docker run --rm -e LICENSE=accept -e COB_CONFIG_DIR=/src/config `
    -e COB_COPY_DIR=/src/copy -e COB_CFLAGS=-I/src `
    -e COB_LDFLAGS=-L/src/build/libcob/.libs `
    -e LD_LIBRARY_PATH=/src/build/libcob/.libs `
    --entrypoint sh -v "${workspace}:/src" -w /src $CobolImage -c $compile
if ($LASTEXITCODE -ne 0) { throw 'ABEND handler worker compilation failed' }

docker run --rm -e LICENSE=accept `
    -e LD_LIBRARY_PATH=/src/build/libcob/.libs `
    --entrypoint sh -v "${workspace}:/src" -w /src $CobolImage -c `
    'python3 extras/cobcics/test-abend-handler.py build/bank-business/workers/abend-handler-worker'
if ($LASTEXITCODE -ne 0) { throw 'ABEND handler runtime test failed' }
