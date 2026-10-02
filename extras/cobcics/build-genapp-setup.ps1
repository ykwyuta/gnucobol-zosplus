param([string]$CobolImage = 'gnucobol-db2-build')

$ErrorActionPreference = 'Stop'
$workspace = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
Set-Location $workspace

python extras/cobcics/audit-genapp.py
if ($LASTEXITCODE -ne 0) { throw 'GenApp translation audit failed' }

docker run --rm -e LICENSE=accept -e COB_CONFIG_DIR=/src/config `
    -e COB_COPY_DIR=/src/copy -e COB_CFLAGS='-I/src -I/usr/local/lib64/python3.9/site-packages/clidriver/include -DCOBCICS_DB2SQL -DCOBCICS_GENAPP' `
    -e 'COB_LDFLAGS=-L/src/build/libcob/.libs -L/src/build/genapp -L/usr/local/lib64/python3.9/site-packages/clidriver/lib -ldb2sql -ldb2 -Wl,-rpath-link,/usr/local/lib64/python3.9/site-packages/ibm_db.libs' `
    --entrypoint sh -v "${workspace}:/src" -w /src $CobolImage extras/cobcics/build-genapp.sh
if ($LASTEXITCODE -ne 0) { throw 'GenApp entry build failed' }
Write-Output "Built $workspace\build\genapp\GENAPP"
