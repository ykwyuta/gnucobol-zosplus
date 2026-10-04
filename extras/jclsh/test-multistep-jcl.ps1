$ErrorActionPreference = 'Stop'
$workspace = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$source = Join-Path $workspace 'extras/jclsh/tests/step.c'
$jcl = Join-Path $workspace 'extras/jclsh/tests/multistep.jcl'
$profile = Join-Path $workspace 'extras/jclsh/tests/multistep.conf'
$binary = Join-Path $workspace 'extras/jclsh/target/debug/jclsh.exe'
$before = (Get-FileHash -LiteralPath $jcl -Algorithm SHA256).Hash

Push-Location $workspace
try {
    cargo build --manifest-path extras/jclsh/Cargo.toml --offline | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'jclsh build failed' }
    New-Item -ItemType Directory -Force build/jclsh | Out-Null
    $mount = ($workspace -replace '\\', '/') + ':/src'
    docker run --rm -v $mount -w /src --entrypoint gcc gnucobol-db2-build -Wall -Wextra -Werror extras/jclsh/tests/step.c -o build/jclsh/step-test
    if ($LASTEXITCODE -ne 0) { throw "test program build failed: $source" }
    $plan = & $binary plan $jcl --profile $profile 2>&1
    if ($LASTEXITCODE -ne 0 -or -not ($plan -match 'STEP CALL.RUN PGM=STEPTEST')) { throw 'PROC expansion failed' }
    $ErrorActionPreference = 'Continue'
    $output = & $binary run $jcl --profile $profile 2>&1
    $ErrorActionPreference = 'Stop'
    if ($LASTEXITCODE -ne 1) { throw "expected job RC 8 and CLI exit 1: $($output -join [Environment]::NewLine)" }
    $summary = $output | Where-Object { $_ -match '^JCLSH JOB MIXJOB RC=8 SPOOL=' } | Select-Object -Last 1
    if (-not $summary -or $summary -notmatch '(MIXJOB-\d+-\d+)$') { throw 'missing job summary' }
    $spool = Join-Path $workspace "build/jclsh/spool/$($Matches[1])"
    $result = Get-Content -LiteralPath (Join-Path $spool 'result.json') -Raw | ConvertFrom-Json
    $statuses = @($result.steps | ForEach-Object { "$($_.step):$($_.status):$($_.rc)" })
    $expected = @('MAKE:EXECUTED:8', 'CALL.CHECK:SKIPPED:', 'CALL.RUN:EXECUTED:0', 'AFTER:EXECUTED:0', 'THEN:EXECUTED:0', 'OTHER:SKIPPED:')
    if ($result.rc -ne 8 -or ($statuses -join '|') -ne ($expected -join '|')) {
        throw "unexpected step results: $($statuses -join ', ')"
    }
    $print = Get-Content -LiteralPath (Join-Path $spool 'CALL.RUN/PRINT.txt') -Raw
    if ($print.Trim() -ne 'PARM=PROCVAL CARD=0') { throw "unexpected PROC output: $print" }
    $card = Get-Content -LiteralPath (Join-Path $spool 'CALL.RUN/CARD.txt') -Raw
    if ($card.Trim() -ne '0') { throw 'PROC DD override failed' }
    $after = (Get-FileHash -LiteralPath $jcl -Algorithm SHA256).Hash
    if ($before -ne $after) { throw 'JCL source changed during execution' }
    Write-Output 'Multistep JCL passed: PROC symbols/DD override, COND, IF/ELSE, RC=8 and spool'
}
finally {
    Pop-Location
}
