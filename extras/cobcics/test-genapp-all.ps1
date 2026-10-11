param(
    [string]$CobolImage = 'gnucobol-db2-build',
    [string]$RustImage = 'rust:1-alpine'
)

$ErrorActionPreference = 'Stop'
$workspace = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
Set-Location $workspace

& extras/cobcics/build-genapp-setup.ps1 -CobolImage $CobolImage
if ($LASTEXITCODE -ne 0) { throw 'GenApp COBOL build failed' }

docker run --rm -v "${workspace}:/src" -w /src/extras/cobcics `
    -e CARGO_TARGET_DIR=/tmp/cobcics-target $RustImage sh -c `
    'cargo build --locked --release --bin cobcicsd && cp /tmp/cobcics-target/release/cobcicsd /src/build/genapp/cobcicsd'
if ($LASTEXITCODE -ne 0) { throw 'GenApp TN3270 host build failed' }

$driver = '/usr/local/lib64/python3.9/site-packages'
$runtime = "LD_LIBRARY_PATH=/src/build/libcob/.libs:/src/build/genapp:$driver/clidriver/lib:$driver/ibm_db.libs"

foreach ($test in @('setup-genapp-file-test-db.py',
                    'setup-genapp-business-db.py')) {
    docker run --rm --env-file extras/db2sql/.runtime.env `
        -v "${workspace}:/src" -w /src --entrypoint python3 `
        $CobolImage "extras/cobcics/$test"
    if ($LASTEXITCODE -ne 0) { throw "$test failed" }
}

docker run --rm --env-file extras/db2sql/.runtime.env `
    -e DB2_SCHEMA=GENAPPTEST -e $runtime `
    -v "${workspace}:/src" -w /src --entrypoint build/genapp/varchar-demo `
    $CobolImage
if ($LASTEXITCODE -ne 0) { throw 'Db2 VARCHAR host variable test failed' }

foreach ($test in @('test-genapp-file.py',
                    'test-genapp-lock.py',
                    'test-genapp-queue.py',
                    'test-genapp-vsam-transactions.py',
                    'test-genapp-event.py',
                    'test-genapp-screens.py',
                    'test-genapp-map5.py',
                    'test-genapp-invalid-input.py',
                    'test-genapp-customer-write.py',
                    'test-genapp-customer-update.py',
                    'test-genapp-rollback.py',
                    'test-genapp-policy-write.py',
                    'test-genapp-policy-update.py',
                    'test-genapp-policy-conflict.py',
                    'test-genapp-other-policy-write.py',
                    'test-genapp-other-policy-update.py',
                    'test-genapp-api-worker.py',
                    'test-genapp-http.py',
                    'test-genapp-scheduler-host.py')) {
    $arguments = @("extras/cobcics/$test")
    if ($test -eq 'test-genapp-file.py') {
        $arguments += 'build/genapp/libgenapp_file.so'
    }
    if ($test -eq 'test-genapp-queue.py') {
        $arguments += 'build/genapp/libgenapp_queue.so'
    }
    if ($test -eq 'test-genapp-vsam-transactions.py') {
        $arguments += 'build/genapp/GENAPP'
    }
    docker run --rm --env-file extras/db2sql/.runtime.env `
        -e DB2_SCHEMA=GENAPPTEST -e $runtime `
        -v "${workspace}:/src" -w /src --entrypoint python3 `
        $CobolImage @arguments
    if ($LASTEXITCODE -ne 0) { throw "$test failed" }
}

Write-Output 'GenApp integrated suite passed'

$cobcBin = if (Test-Path 'build/db2sql-build/cobc/cobc') {
    'build/db2sql-build/cobc/cobc'
} else { 'build/cobc/cobc' }
docker run --rm -e COB_CONFIG_DIR=/src/config -e COB_COPY_DIR=/src/copy `
    -e "COBC_BIN=$cobcBin" `
    -e COB_CFLAGS='-I/src -I/usr/local/lib64/python3.9/site-packages/clidriver/include -DCOBCICS_DB2SQL -DCOBCICS_GENAPP' `
    -e 'COB_LDFLAGS=-L/src/build/libcob/.libs -L/src/build/genapp -L/usr/local/lib64/python3.9/site-packages/clidriver/lib -ldb2sql -ldb2 -Wl,-rpath-link,/usr/local/lib64/python3.9/site-packages/ibm_db.libs' `
    --entrypoint sh -v "${workspace}:/src" -w /src `
    $CobolImage extras/cobcics/build-genapp-ebcdic.sh
if ($LASTEXITCODE -ne 0) { throw 'GenApp IBM-037 build failed' }

foreach ($test in @('test-genapp-ebcdic.py',
                    'test-genapp-ebcdic-file.py',
                    'test-genapp-screens.py',
                    'test-genapp-invalid-input.py',
                    'test-genapp-customer-write.py',
                    'test-genapp-customer-update.py',
                    'test-genapp-policy-write.py',
                    'test-genapp-policy-update.py',
                    'test-genapp-policy-conflict.py',
                    'test-genapp-other-policy-write.py',
                    'test-genapp-other-policy-update.py')) {
    docker run --rm --env-file extras/db2sql/.runtime.env `
        -e DB2_SCHEMA=GENAPPTEST -e COB_HOST_CODESET=UTF-8 `
        -e GENAPP_BUILD_DIR=build/genapp-ebcdic -e $runtime `
        -v "${workspace}:/src" -w /src --entrypoint python3 `
        $CobolImage "extras/cobcics/$test"
    if ($LASTEXITCODE -ne 0) { throw "IBM-037 $test failed" }
}

Write-Output 'GenApp IBM-037 screen, SQL and FILE suite passed'
