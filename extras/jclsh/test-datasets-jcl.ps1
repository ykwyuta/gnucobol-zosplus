$ErrorActionPreference = 'Stop'
$workspace = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$binary = Join-Path $workspace 'extras/jclsh/target/debug/jclsh.exe'
$profile = Join-Path $workspace 'extras/jclsh/tests/datasets.conf'
$datasetsJcl = Join-Path $workspace 'extras/jclsh/tests/datasets.jcl'
$abendJcl = Join-Path $workspace 'extras/jclsh/tests/abend.jcl'
$target = Join-Path $workspace 'build/jclsh/datasets/test-out.dat'
$archiveTarget = Join-Path $workspace 'build/jclsh/datasets/test-arch.dat'
$archive = Join-Path $workspace 'build/jclsh/datasets/.uncataloged/TEST_ARCH'
$before = @((Get-FileHash -LiteralPath $datasetsJcl -Algorithm SHA256).Hash, (Get-FileHash -LiteralPath $abendJcl -Algorithm SHA256).Hash)

Push-Location $workspace
try {
    cargo build --manifest-path extras/jclsh/Cargo.toml --offline | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'jclsh build failed' }
    New-Item -ItemType Directory -Force build/jclsh/datasets | Out-Null
    $mount = ($workspace -replace '\\', '/') + ':/src'
    docker run --rm -v $mount -w /src --entrypoint gcc gnucobol-db2-build -Wall -Wextra -Werror extras/jclsh/tests/step.c -o build/jclsh/step-test
    if ($LASTEXITCODE -ne 0) { throw 'test program build failed' }
    if (Test-Path -LiteralPath $target) { Remove-Item -LiteralPath $target -Force }
    if (Test-Path -LiteralPath $archiveTarget) { Remove-Item -LiteralPath $archiveTarget -Force }
    if (Test-Path -LiteralPath $archive) { Remove-Item -LiteralPath $archive -Force }
    $output = & $binary run $datasetsJcl --profile $profile 2>&1
    if ($LASTEXITCODE -ne 0) { throw "dataset job failed: $($output -join [Environment]::NewLine)" }
    $summary = $output | Where-Object { $_ -match '^JCLSH JOB DATAJOB RC=0 SPOOL=' } | Select-Object -Last 1
    if (-not $summary -or $summary -notmatch '(DATAJOB-\d+-\d+)$') { throw 'missing dataset job summary' }
    $spool = Join-Path $workspace "build/jclsh/spool/$($Matches[1])"
    $result = Get-Content -LiteralPath (Join-Path $spool 'result.json') -Raw | ConvertFrom-Json
    if ($result.rc -ne 0 -or $result.steps.Count -ne 6 -or @($result.steps | Where-Object { $_.status -ne 'EXECUTED' }).Count) { throw 'unexpected dataset step results' }
    if (-not (Get-Content (Join-Path $spool 'COPY/PRINT.txt') -Raw).Contains('INPUT=CATALOG')) { throw 'PASS did not reach COPY' }
    if (-not (Get-Content (Join-Path $spool 'CHECK/PRINT.txt') -Raw).Contains('INPUT=PERSIST')) { throw 'CATLG did not reach CHECK' }
    if (Test-Path -LiteralPath $target) { throw 'DISP=DELETE left cataloged output' }
    if (Test-Path -LiteralPath $archiveTarget) { throw 'DISP=UNCATLG left cataloged output' }
    if (-not (Test-Path -LiteralPath $archive) -or (Get-Content -LiteralPath $archive -Raw).Trim() -ne 'ARCHIVE') { throw 'UNCATLG did not preserve the uncataloged data' }
    if (Get-ChildItem $spool -Recurse -File -Filter 'dataset-*.dat') { throw 'temporary datasets were not deleted' }

    $ErrorActionPreference = 'Continue'
    $output = & $binary run $abendJcl --profile $profile 2>&1
    $ErrorActionPreference = 'Stop'
    if ($LASTEXITCODE -ne 1) { throw 'ABEND job should return CLI exit 1' }
    $summary = $output | Where-Object { $_ -match '^JCLSH JOB ABJOB RC=16 SPOOL=' } | Select-Object -Last 1
    if (-not $summary -or $summary -notmatch '(ABJOB-\d+-\d+)$') { throw 'missing ABEND job summary' }
    $spool = Join-Path $workspace "build/jclsh/spool/$($Matches[1])"
    $result = Get-Content -LiteralPath (Join-Path $spool 'result.json') -Raw | ConvertFrom-Json
    $statuses = @($result.steps | ForEach-Object { "$($_.step):$($_.status)" })
    $expected = @('FAIL:ABEND', 'SKIP:SKIPPED', 'ONLY:EXECUTED', 'EVEN:EXECUTED', 'BRANCH:EXECUTED')
    if ($result.rc -ne 16 -or ($statuses -join '|') -ne ($expected -join '|') -or $result.steps[0].abend -ne 'U1234') {
        throw "unexpected ABEND results: $($statuses -join ', ')"
    }
    if (Get-ChildItem $spool -Recurse -File -Filter 'dataset-*.dat') { throw 'ABEND DISP=DELETE left a dataset' }
    $after = @((Get-FileHash -LiteralPath $datasetsJcl -Algorithm SHA256).Hash, (Get-FileHash -LiteralPath $abendJcl -Algorithm SHA256).Hash)
    if (($before -join '|') -ne ($after -join '|')) { throw 'source JCL changed' }
    Write-Output 'Dataset/ABEND JCL passed: cataloged PROC, INCLUDE, PASS, CATLG, DELETE, ONLY, EVEN and IF ABEND'
}
finally {
    Pop-Location
}
