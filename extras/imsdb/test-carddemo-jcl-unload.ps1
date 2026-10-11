$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
Push-Location $root
try {
    New-Item -ItemType Directory -Force -Path build/imsdb-carddemo-jcl | Out-Null
    wsl --exec sh /mnt/d/workspace/gnucobol/extras/imsdb/test-carddemo-jcl-unload-prepare.sh
    if ($LASTEXITCODE -ne 0) { throw 'IMS original unload setup failed' }
    $jcl = 'reference/aws-mainframe-modernization-carddemo/app/app-authorization-ims-db2-mq/jcl/UNLDPADB.JCL'
    $profile = 'extras/jclsh/tests/ims-carddemo-unload.conf'
    $plan = cargo run --manifest-path extras/jclsh/Cargo.toml --offline -- plan $jcl --profile $profile
    if ($LASTEXITCODE -ne 0 -or ($plan -join "`n") -notmatch 'IMS DLI PROGRAM=PAUDBUNL PSB=PAUTBUNL') {
        throw 'CardDemo original unload JCL plan failed'
    }
    $result = cargo run --manifest-path extras/jclsh/Cargo.toml --offline -- run $jcl --profile $profile
    if ($LASTEXITCODE -ne 0 -or $result -notcontains 'JCLSH STEP STEP01 RC=0') {
        throw 'CardDemo original unload JCL execution failed'
    }
    $out = Join-Path $root 'build/imsdb-carddemo-jcl'
    $expectedRoots = [IO.File]::ReadAllBytes((Join-Path $out 'expected-roots.fb'))
    $actualRoots = [IO.File]::ReadAllBytes((Join-Path $out 'roots.fb'))
    $expectedChildren = [IO.File]::ReadAllBytes((Join-Path $out 'expected-children.fb'))
    $actualChildren = [IO.File]::ReadAllBytes((Join-Path $out 'children.fb'))
    if ($expectedRoots.Length -ne 2200 -or $actualRoots.Length -ne 2100 -or
        $expectedChildren.Length -ne 202 * 206 -or
        -not [Linq.Enumerable]::SequenceEqual([byte[]]$actualRoots, [byte[]]$expectedRoots[0..2099]) -or
        -not [Linq.Enumerable]::SequenceEqual([byte[]]$actualChildren, [byte[]]$expectedChildren)) {
        throw 'CardDemo original unload JCL file bytes differ'
    }
    $result | Write-Output
    'IMS CARDDEMO ORIGINAL JCL UNLOAD OK'
} finally {
    Pop-Location
}
