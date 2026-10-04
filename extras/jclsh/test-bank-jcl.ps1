$ErrorActionPreference = 'Stop'
$workspace = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$jcl = Join-Path $workspace 'reference/Bank-of-Z-main/src/base/batch/jcl/BNKSTMT.jcl'
$profile = Join-Path $workspace 'extras/jclsh/bank.conf'
$binary = Join-Path $workspace 'extras/jclsh/target/debug/jclsh.exe'
$before = (Get-FileHash -LiteralPath $jcl -Algorithm SHA256).Hash

Push-Location $workspace
try {
    cargo build --manifest-path extras/jclsh/Cargo.toml --offline | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'jclsh build failed' }
    $output = & $binary run $jcl --profile $profile 2>&1
    if ($LASTEXITCODE -ne 0) { throw "jclsh failed: $($output -join [Environment]::NewLine)" }
    $summary = $output | Where-Object { $_ -match '^JCLSH JOB BNKSTMT RC=0 SPOOL=' } | Select-Object -Last 1
    if (-not $summary -or $summary -notmatch '(BNKSTMT-\d+-\d+)$') {
        throw 'missing successful job summary'
    }
    $jobId = $Matches[1]
    $spool = Join-Path $workspace "build/jclsh/spool/$jobId"
    $result = Get-Content -LiteralPath (Join-Path $spool 'result.json') -Raw | ConvertFrom-Json
    if ($result.rc -ne 0 -or $result.steps.Count -ne 1 -or $result.steps[0].rc -ne 0) {
        throw 'unexpected step or job RC'
    }
    $report = Get-Content -LiteralPath (Join-Path $spool 'STEP1/SYSPRINT.txt') -Raw
    $stderr = Get-Content -LiteralPath (Join-Path $spool 'STEP1/SYSTSPRT.txt') -Raw
    if ($stderr) { throw "unexpected SYSTSPRT: $stderr" }
    foreach ($expected in @(
        'REPORTING MONTH FROM DATECARD:  202606',
        'STATEMENT PERIOD:  20260601  TO  20260630',
        'ACCOUNT NUMBER: 123456-00000001',
        'ACCOUNT NUMBER: 123456-00000002',
        '2026-06-05 09:15:30',
        '2026-06-10 14:30:00',
        'OPENING BALANCE:        $   1000.00',
        'CLOSING BALANCE:        $   1100.00',
        'TOTAL STATEMENTS GENERATED:  2'
    )) {
        if (-not $report.Contains($expected)) { throw "report is missing: $expected" }
    }
    if ($report.Contains('PL1OLD000003')) { throw 'prior-month transaction appeared in report' }
    if ($report.Split([char]12).Count -ne 3) { throw 'expected two form feeds' }
    if ((Get-Content -LiteralPath (Join-Path $spool 'STEP1/DATECARD.txt') -Raw).Trim() -ne '202606') {
        throw 'DATECARD was not passed to the step'
    }
    if ((Get-Content -LiteralPath (Join-Path $spool 'STEP1/SORTCODE.txt') -Raw).Trim() -ne '123456') {
        throw 'SORTCODE was not passed to the step'
    }
    $after = (Get-FileHash -LiteralPath $jcl -Algorithm SHA256).Hash
    if ($before -ne $after) { throw 'source JCL changed during execution' }
    Write-Output "BNKSTMT JCL passed: RC=0, 2 accounts, 2 June transactions, DD and spool ($jobId)"
}
finally {
    Pop-Location
}
