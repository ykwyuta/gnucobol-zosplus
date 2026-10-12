param(
    [string]$CobolImage = 'gnucobol-db2-build',
    [string]$Db2Port = '50001'
)

$ErrorActionPreference = 'Stop'
$workspace = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
Set-Location $workspace
if (-not (Test-Path extras/db2sql/.runtime.env)) {
    throw 'Create extras/db2sql/.runtime.env before running this test'
}
New-Item -ItemType Directory -Force build/carddemo-fraud | Out-Null
$compile = @'
set -eu
out=build/carddemo-fraud
compiler="$out/compiler"
card=reference/aws-mainframe-modernization-carddemo/app/app-authorization-ims-db2-mq
driver=/usr/local/lib64/python3.9/site-packages
if [ ! -x "$compiler/cobc/cobc" ]; then
  mkdir -p "$compiler"
  (cd "$compiler" && /src/configure --without-db --prefix="/src/$compiler/install" \
    && make tarstamp.h && make -C lib -j4 \
    && make -C libcob -j4 && make -C cobc cobc)
fi
cp "$card/dcl/AUTHFRDS.dcl" "$out/AUTHFRDS.cpy"
sed 's/EXEC CICS ASKTIME NOHANDLE/EXEC CICS ASKTIME         /' \
  "$card/cbl/COPAUS2C.cbl" > "$out/COPAUS2C.cbl"
gcc -std=c11 -Wall -Wextra -Werror -fPIC -shared \
  -I/src -I"$driver/clidriver/include" extras/db2sql/db2sql.c \
  -L/src/$compiler/libcob/.libs -lcob \
  -L"$driver/clidriver/lib" -ldb2 \
  -o "$out/libdb2sql.so"
for encoding in native ebcdic; do
  set --
  if [ "$encoding" = ebcdic ]; then
    set -- -fdata-codeset=IBM-037 -fbinary-byteorder=big-endian
  fi
  "$compiler/cobc/cobc" "$@" \
    --cics-map="$card/cpy-bms/COPAU00.cpy" --db2-sql \
    -c -I "$out" -I "$card/cpy" -I "$card/cpy-bms" \
    -I reference/aws-mainframe-modernization-carddemo/app/cpy \
    -I extras/cobcics/copy -o "$out/COPAUS2C-$encoding.o" "$out/COPAUS2C.cbl"
  "$compiler/cobc/cobc" "$@" -x -I "$card/cpy" \
    -o "$out/fraud-worker-$encoding" \
    extras/imsdb/tests/carddemo-fraud-worker.cob \
    "$out/COPAUS2C-$encoding.o" extras/cobcics/bridge.c
done
'@
$driver = '/usr/local/lib64/python3.9/site-packages'
docker run --rm -e LICENSE=accept -e COB_CONFIG_DIR=/src/config `
    -e COB_COPY_DIR=/src/copy `
    -e COB_CICS_PREPROCESSOR=/src/build/preprocessors-linux/debug/cics-preprocess `
    -e COB_DB2_SQL_PREPROCESSOR=/src/build/preprocessors-linux/debug/db2sql-preprocess `
    -e COB_CFLAGS='-I/src -DCOBCICS_DB2SQL' `
    -e "COB_LDFLAGS=-L/src/build/carddemo-fraud/compiler/libcob/.libs -L/src/build/carddemo-fraud -ldb2sql -Wl,-rpath-link,$driver/ibm_db.libs" `
    -e "LD_LIBRARY_PATH=/src/build/carddemo-fraud/compiler/libcob/.libs:/src/build/carddemo-fraud:$driver/clidriver/lib:$driver/ibm_db.libs" `
    --entrypoint sh -v "${workspace}:/src" -w /src $CobolImage -c $compile
if ($LASTEXITCODE -ne 0) { throw 'CardDemo fraud worker build failed' }

docker run --rm --env-file extras/db2sql/.runtime.env -e LICENSE=accept `
    -e "DB2_PORT=$Db2Port" `
    -e COB_HOST_CODESET=UTF-8 `
    -e "LD_LIBRARY_PATH=/src/build/carddemo-fraud/compiler/libcob/.libs:/src/build/carddemo-fraud:$driver/clidriver/lib:$driver/ibm_db.libs" `
    --entrypoint python3 -v "${workspace}:/src" -w /src $CobolImage `
    extras/imsdb/test-carddemo-fraud.py `
    build/carddemo-fraud/fraud-worker-native `
    build/carddemo-fraud/fraud-worker-ebcdic
if ($LASTEXITCODE -ne 0) { throw 'CardDemo fraud Db2 integration failed' }
