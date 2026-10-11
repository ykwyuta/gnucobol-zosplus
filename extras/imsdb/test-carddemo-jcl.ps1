$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
Push-Location $root
try {
    $mount = ($root -replace '\\', '/') + ':/src'
    New-Item -ItemType Directory -Force -Path build/imsdb-carddemo | Out-Null
    docker build -q -t gnucobol-ims-integration -f extras/imsdb/Dockerfile.integration .
    if ($LASTEXITCODE -ne 0) { throw 'IMS integration image build failed' }
    docker run --rm --network none -v $mount -w /src gnucobol-ims-integration `
        make -s -C build/libcob -j2
    if ($LASTEXITCODE -ne 0) { throw 'libcob build failed' }
    docker run --rm --network none -v $mount -w /src gnucobol-ims-integration `
        make -s -C build/cobc cobc
    if ($LASTEXITCODE -ne 0) { throw 'cobc build failed' }
    docker run --rm --network none -v $mount -w /src `
        -e CARGO_TARGET_DIR=/src/build/imsdb-linux rust:1-bookworm `
        sh -lc 'PATH=/usr/local/cargo/bin:$PATH cargo build --manifest-path extras/imsdb/Cargo.toml --offline --lib --bin imsdbctl'
    if ($LASTEXITCODE -ne 0) { throw 'IMS Rust build failed' }
    wsl --exec sh /mnt/d/workspace/gnucobol/extras/imsdb/test-carddemo-execdli.sh
    if ($LASTEXITCODE -ne 0) { throw 'CardDemo COBOL build and direct run failed' }
    $jcl = 'reference/aws-mainframe-modernization-carddemo/app/app-authorization-ims-db2-mq/jcl/CBPAUP0J.jcl'
    $profile = 'extras/jclsh/tests/ims-carddemo.conf'
    $plan = cargo run --manifest-path extras/jclsh/Cargo.toml --offline -- plan $jcl --profile $profile
    if ($LASTEXITCODE -ne 0 -or ($plan -join "`n") -notmatch 'IMS BMP PROGRAM=CBPAUP0C PSB=PSBPAUTB') {
        throw 'CardDemo original JCL plan failed'
    }
    $result = cargo run --manifest-path extras/jclsh/Cargo.toml --offline -- run $jcl --profile $profile
    if ($LASTEXITCODE -ne 0 -or ($result -join "`n") -notmatch '# TOTAL SUMMARY READ' -or `
        $result -notcontains 'JCLSH STEP STEP01 RC=0') {
        throw 'CardDemo original JCL execution failed'
    }
    $result | Write-Output
} finally {
    Pop-Location
}
