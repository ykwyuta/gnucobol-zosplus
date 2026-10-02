param([switch]$SkipAudit)

$ErrorActionPreference = 'Stop'
$workspace = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
Set-Location $workspace
if (-not $SkipAudit) {
    python extras/cobcics/audit-bank-syntax.py
    if ($LASTEXITCODE -ne 0) { throw 'Bank syntax audit failed' }
    & extras/cobcics/build-bank-business.ps1 *> build/bank-syntax/build-business.log
    if ($LASTEXITCODE -ne 0) { throw 'Bank business build failed' }
}

$driver = '/usr/local/lib64/python3.9/site-packages'
$link = @'
set -eu
mkdir -p build/bank-syntax/screens
all=''
for file in build/bank-syntax/*.o; do
  case "$file" in *BANKDATA.o) ;; *) all="$all $file" ;; esac
done
build/cobc/cobc -x -I reference/Bank-of-Z-main/src/base/cics/copy \
  -I extras/cobcics/copy -I build/bank-syntax/maps \
  -o build/bank-syntax/screens/BNKLINK \
  extras/cobcics/bank-linkage-screen.cob $all extras/cobcics/bridge.c \
  >build/bank-syntax/screens/BNKLINK.log 2>&1 || {
    tail -30 build/bank-syntax/screens/BNKLINK.log
    exit 1
  }
for name in BNKMENU BNK1DAC BNK1TFN BNK1CCS BNK1CAC BNK1UAC BNK1CCA; do
  others=''
  for file in build/bank-syntax/*.o; do
    case "$file" in *BANKDATA.o|*/$name.o) ;; *) others="$others $file" ;; esac
  done
  build/cobc/cobc -x -I reference/Bank-of-Z-main/src/base/cics/copy \
    -I extras/cobcics/copy -I build/bank-syntax/maps \
    -o "build/bank-syntax/screens/$name" "build/bank-syntax/sql/$name.cob" \
    $others extras/cobcics/bridge.c \
    >"build/bank-syntax/screens/$name.log" 2>&1 || {
      tail -30 "build/bank-syntax/screens/$name.log"
      exit 1
    }
  echo "$name linked"
done
'@
docker run --rm -e LICENSE=accept -e COB_CONFIG_DIR=/src/config `
    -e COB_COPY_DIR=/src/copy -e COB_CFLAGS='-I/src -DCOBCICS_DB2SQL' `
    -e "COB_LDFLAGS=-L/src/build/libcob/.libs -L/src/build/bank-business -ldb2sql -L$driver/clidriver/lib -ldb2 -Wl,-rpath-link,$driver/ibm_db.libs" `
    --entrypoint sh -v "${workspace}:/src" -w /src gnucobol-db2-build -c $link
if ($LASTEXITCODE -ne 0) { throw 'Bank screen link failed' }
