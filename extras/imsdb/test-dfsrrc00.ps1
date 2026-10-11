$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
Push-Location $root
try {
    $mount = ($root -replace '\\', '/') + ':/src'
    $libPath = '/src/build/libcob/.libs:/src/build/imsdb-linux/debug'
    docker build -q -t gnucobol-ims-integration -f extras/imsdb/Dockerfile.integration .
    if ($LASTEXITCODE -ne 0) { throw 'IMS integration image build failed' }
    docker run --rm --network none -v $mount -w /src gnucobol-ims-integration `
        make -s -C build/libcob -j2
    if ($LASTEXITCODE -ne 0) { throw 'libcob build failed' }
    docker run --rm --network none -v $mount -w /src -e CARGO_TARGET_DIR=/src/build/imsdb-linux rust:1-bookworm `
        sh -lc 'PATH=/usr/local/cargo/bin:$PATH cargo build --manifest-path extras/imsdb/Cargo.toml --offline --lib --bin imsdbctl'
    if ($LASTEXITCODE -ne 0) { throw 'IMS Rust build failed' }
    docker run --rm --network none -v $mount -w /src -e "LD_LIBRARY_PATH=$libPath" `
        -e COB_CONFIG_DIR=/src/config -e COB_COPY_DIR=/src/copy `
        -e COB_CFLAGS=-I/src -e COB_LDFLAGS=-L/src/build/libcob/.libs `
        gnucobol-ims-integration sh -lc `
        'gcc -std=c11 -Wall -Wextra -Werror -fPIC -shared -I. -Iextras/imsdb extras/imsdb/cbltdli.c -Lbuild/imsdb-linux/debug -limsdb -Lbuild/libcob/.libs -lcob -pthread -o build/imsdb-linux/libcobims.so && build/cobc/.libs/cobc -x -fdata-codeset=IBM-037 -fbinary-byteorder=big-endian -o build/imsdb-linux/ims-smoke extras/imsdb/tests/cbltdli-smoke.cob'
    if ($LASTEXITCODE -ne 0) { throw 'IMS COBOL build failed' }
    $wal = Join-Path $root 'build/imsdb-linux/jcl-smoke.wal'
    if (Test-Path -LiteralPath $wal) { Remove-Item -LiteralPath $wal -Force }
    docker run --rm --network none -v $mount -w /src `
        -e LD_LIBRARY_PATH=/src/build/imsdb-linux/debug gnucobol-ims-integration `
        build/imsdb-linux/debug/imsdbctl import-csv-bundle `
        build/imsdb-linux/jcl-smoke.wal reference/Bank-of-Z-main/src/base/ims/DBD `
        reference/Bank-of-Z-main/src/base/ims/LoadData
    if ($LASTEXITCODE -ne 0) { throw 'Bank of Z IMS load failed' }
    cargo run --manifest-path extras/jclsh/Cargo.toml --offline -- plan `
        extras/jclsh/tests/ims-dfsrrc00.jcl --profile extras/jclsh/tests/ims-dfsrrc00.conf
    if ($LASTEXITCODE -ne 0) { throw 'DFSRRC00 plan failed' }
    $output = cargo run --manifest-path extras/jclsh/Cargo.toml --offline -- run `
        extras/jclsh/tests/ims-dfsrrc00.jcl --profile extras/jclsh/tests/ims-dfsrrc00.conf
    if ($LASTEXITCODE -ne 0 -or $output -notcontains 'IMS CUSTOMER UPDATE OK' -or `
        $output -notcontains 'JCLSH STEP STEP01 RC=0') { throw 'DFSRRC00 update failed' }
    $output | Write-Output
    $output = cargo run --manifest-path extras/jclsh/Cargo.toml --offline -- run `
        extras/jclsh/tests/ims-dfsrrc00.jcl --profile extras/jclsh/tests/ims-dfsrrc00-auto.conf
    if ($LASTEXITCODE -ne 0 -or $output -notcontains 'IMS CUSTOMER AUTO UPDATE OK' -or `
        $output -notcontains 'JCLSH STEP STEP01 RC=0') { throw 'DFSRRC00 automatic commit failed' }
    $output | Write-Output
    $output = docker run --rm --network none -v $mount -w /src -e "LD_LIBRARY_PATH=$libPath" `
        -e COB_PRE_LOAD=/src/build/imsdb-linux/libcobims.so `
        -e COB_IMS_WAL=/src/build/imsdb-linux/jcl-smoke.wal `
        -e COB_IMS_DBD_DIR=/src/reference/Bank-of-Z-main/src/base/ims/DBD `
        -e COB_IMS_PSB=/src/reference/Bank-of-Z-main/src/base/ims/PSB/IBSCUDAT.asm `
        -e COB_IMS_VERIFY_ONLY=4 gnucobol-ims-integration /src/build/imsdb-linux/ims-smoke
    if ($LASTEXITCODE -ne 0 -or $output -notcontains 'IMS CUSTOMER AUTO REOPEN OK') {
        throw 'DFSRRC00 committed data was not persisted'
    }
    $output | Write-Output
} finally {
    Pop-Location
}
