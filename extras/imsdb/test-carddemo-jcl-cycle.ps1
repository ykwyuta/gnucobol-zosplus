$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
Push-Location $root
try {
    & (Join-Path $PSScriptRoot 'test-carddemo-jcl-unload.ps1')
    if ($LASTEXITCODE -ne 0) { throw 'CardDemo original unload JCL failed' }
    $jcl = 'reference/aws-mainframe-modernization-carddemo/app/app-authorization-ims-db2-mq/jcl/LOADPADB.JCL'
    $profile = 'extras/jclsh/tests/ims-carddemo-load.conf'
    $plan = cargo run --manifest-path extras/jclsh/Cargo.toml --offline -- plan $jcl --profile $profile
    if ($LASTEXITCODE -ne 0 -or ($plan -join "`n") -notmatch 'IMS BMP PROGRAM=PAUDBLOD PSB=PSBPAUTB') {
        throw 'CardDemo original load JCL plan failed'
    }
    $result = cargo run --manifest-path extras/jclsh/Cargo.toml --offline -- run $jcl --profile $profile
    if ($LASTEXITCODE -ne 0 -or $result -notcontains 'JCLSH STEP STEP01 RC=0') {
        throw 'CardDemo original load JCL execution failed'
    }
    $mount = ($root -replace '\\', '/') + ':/src'
    $count = docker run --rm --network none -v $mount -w /src `
        --entrypoint /src/build/imsdb-linux/debug/imsdbctl `
        gnucobol-ims-integration count /src/build/imsdb-carddemo-jcl/load.wal `
        /src/reference/aws-mainframe-modernization-carddemo/app/app-authorization-ims-db2-mq/ims/DBPAUTP0.dbd
    if ($LASTEXITCODE -ne 0 -or ($count -join "`n").Trim() -ne '223') {
        throw 'CardDemo original load JCL did not persist 21 roots and 202 children'
    }
    $result | Select-String '^JCLSH STEP|^JCLSH JOB' | ForEach-Object Line
    'IMS CARDDEMO ORIGINAL JCL LOAD 223 SEGMENTS OK'
} finally {
    Pop-Location
}
