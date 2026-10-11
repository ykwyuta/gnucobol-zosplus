$ErrorActionPreference = 'Stop'
$workspace = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
Push-Location $workspace
try {
    $root = Join-Path $workspace 'build/jclsh/utility-datasets'
    New-Item -ItemType Directory -Force $root | Out-Null
    foreach ($name in @('input.dat', 'sorted.dat', 'copy.dat')) {
        $path = Join-Path $root $name
        if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path }
    }
    cargo run --locked --manifest-path extras/jclsh/Cargo.toml -- run extras/jclsh/tests/utilities.jcl --profile extras/jclsh/tests/utilities.conf
    if ($LASTEXITCODE -ne 0) { throw 'utility JCL failed' }
    $expected = "A01`nB02`nC03`n"
    if ((Get-Content -LiteralPath (Join-Path $root 'sorted.dat') -Raw) -ne $expected) {
        throw 'SORT output differs'
    }
    if (Test-Path -LiteralPath (Join-Path $root 'copy.dat')) {
        throw 'IEFBR14 did not delete TEST.COPY'
    }
    foreach ($name in @('symbolic-output.dat', 'symbolic-report.dat')) {
        $path = Join-Path $root $name
        if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path }
    }
    function New-TestRecord([string]$card, [string]$date) {
        $chars = [char[]](' ' * 350)
        $card.CopyTo(0, $chars, 262, 16)
        $date.CopyTo(0, $chars, 304, 10)
        return [string]::new($chars)
    }
    $records = @(
        (New-TestRecord '0000000000000002' '2022-06-20'),
        (New-TestRecord '0000000000000003' '2022-05-20'),
        (New-TestRecord '0000000000000001' '2022-06-10')
    )
    [System.IO.File]::WriteAllText((Join-Path $root 'symbolic-input.dat'), (($records -join "`n") + "`n"), [System.Text.Encoding]::ASCII)
    cargo run --locked --manifest-path extras/jclsh/Cargo.toml -- run extras/jclsh/tests/sort-symbolic.jcl --profile extras/jclsh/tests/utilities.conf
    if ($LASTEXITCODE -ne 0) { throw 'symbolic SORT JCL failed' }
    $sorted = [System.IO.File]::ReadAllLines((Join-Path $root 'symbolic-output.dat'))
    $report = [System.IO.File]::ReadAllLines((Join-Path $root 'symbolic-report.dat'))
    if ($sorted.Count -ne 2 -or $sorted[0].Substring(262, 16) -ne '0000000000000001' -or $sorted[1].Substring(262, 16) -ne '0000000000000002') {
        throw 'symbolic SORT filter or key order differs'
    }
    if ($report.Count -ne 2 -or $report[0].Substring(0, 16) -ne '0000000000000001' -or $report[0].Length -ne 328) {
        throw 'OUTREC reformat differs'
    }
    $fbRoot = Join-Path $workspace 'build/jclsh/fb-sort'
    New-Item -ItemType Directory -Force $fbRoot | Out-Null
    [System.IO.File]::WriteAllBytes((Join-Path $fbRoot 'input.dat'), [System.Text.Encoding]::ASCII.GetBytes('B002A001'))
    foreach ($path in @('input.dat.jclsh-meta', 'output.dat', 'output.dat.jclsh-meta')) {
        $target = Join-Path $fbRoot $path
        if (Test-Path -LiteralPath $target) { Remove-Item -LiteralPath $target }
    }
    cargo run --locked --manifest-path extras/jclsh/Cargo.toml -- run extras/jclsh/tests/fb-sort.jcl --profile extras/jclsh/tests/fb-sort.conf
    if ($LASTEXITCODE -ne 0) { throw 'FB SORT JCL failed' }
    if ([System.Text.Encoding]::ASCII.GetString([System.IO.File]::ReadAllBytes((Join-Path $fbRoot 'output.dat'))) -ne 'A001B002' -or
        (Get-Content -LiteralPath (Join-Path $fbRoot 'output.dat.jclsh-meta') -Raw) -notmatch '^FB\s+4\s+0\s+0') {
        throw 'FB SORT record or metadata differs'
    }
    $cardRoot = Join-Path $workspace 'build/jclsh/carddemo-datasets'
    New-Item -ItemType Directory -Force $cardRoot | Out-Null
    foreach ($name in @('user-security-ps.dat', 'user-security-esds.dat', 'user-security-rrds.dat')) {
        $path = Join-Path $cardRoot $name
        if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path }
    }
    cargo run --locked --manifest-path extras/jclsh/Cargo.toml -- run reference/aws-mainframe-modernization-carddemo/app/jcl/ESDSRRDS.jcl --profile extras/jclsh/tests/carddemo-utilities.conf
    if ($LASTEXITCODE -ne 0) { throw 'original CardDemo JCL failed' }
    $source = [System.IO.File]::ReadAllBytes((Join-Path $cardRoot 'user-security-ps.dat'))
    foreach ($name in @('user-security-esds.dat', 'user-security-rrds.dat')) {
        $actual = [System.IO.File]::ReadAllBytes((Join-Path $cardRoot $name))
        if (-not [System.Collections.StructuralComparisons]::StructuralEqualityComparer.Equals($source, $actual)) {
            throw "IDCAMS REPRO output differs: $name"
        }
    }
    if ($source.Length -ne 800 -or [System.Text.Encoding]::ASCII.GetString($source, 0, 8) -ne 'ADMIN001') {
        throw 'CardDemo FB input record count or content differs'
    }
    $csdRoot = Join-Path $workspace 'build/jclsh/genapp-csd'
    New-Item -ItemType Directory -Force $csdRoot | Out-Null
    [System.IO.File]::WriteAllText((Join-Path $csdRoot 'catalog.dat'), '')
    foreach ($member in @('cdef121', 'cdef122', 'cdef123', 'cdef124', 'cdef125')) {
        cargo run --locked --manifest-path extras/jclsh/Cargo.toml -- run "reference/cics-genapp/base/cntl/$member.jcl" --profile extras/jclsh/tests/genapp-csd.conf
        if ($LASTEXITCODE -ne 0) { throw "original GenApp DFHCSDUP JCL failed: $member" }
    }
    $catalog = Get-Content -LiteralPath (Join-Path $csdRoot 'catalog.dat')
    if (-not ($catalog | Where-Object { $_ -match '^R\s+GENASAT\s+TRANSACTION\s+SSC1\s+' })) {
        throw 'GenApp CSD transaction SSC1 missing'
    }
    if (-not ($catalog | Where-Object { $_ -match '^L\s+GENALIST\s+GENASAT$' })) {
        throw 'GenApp CSD group list membership missing'
    }
    $vsamRoot = Join-Path $workspace 'build/jclsh/genapp-vsam'
    New-Item -ItemType Directory -Force $vsamRoot | Out-Null
    [System.IO.File]::WriteAllText((Join-Path $vsamRoot 'source-customer.dat'), "CUST000001`n")
    [System.IO.File]::WriteAllText((Join-Path $vsamRoot 'source-policy.dat'), "POLICY000001`n")
    foreach ($name in @('ksdscust.dat', 'ksdspoly.dat')) {
        $path = Join-Path $vsamRoot $name
        if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path }
    }
    cargo run --locked --manifest-path extras/jclsh/Cargo.toml -- run reference/cics-genapp/base/cntl/adef121.jcl --profile extras/jclsh/tests/genapp-vsam.conf
    if ($LASTEXITCODE -ne 0) { throw 'original GenApp IDCAMS JCL failed' }
    $customerBytes = [System.IO.File]::ReadAllBytes((Join-Path $vsamRoot 'ksdscust.dat'))
    $policyBytes = [System.IO.File]::ReadAllBytes((Join-Path $vsamRoot 'ksdspoly.dat'))
    if ($customerBytes.Length -ne 225 -or $policyBytes.Length -ne 64 -or
        [System.Text.Encoding]::ASCII.GetString($customerBytes, 0, 10) -ne 'CUST000001' -or
        [System.Text.Encoding]::ASCII.GetString($policyBytes, 0, 12) -ne 'POLICY000001') {
        throw 'GenApp IDCAMS REPRO output differs'
    }
    $csdCheck = & cargo run --quiet --locked --manifest-path extras/cobcics/Cargo.toml --bin cobcicsd -- --bms reference/Bank-of-Z-main/src/base/cics/bms/BNK1MAI.bms --csd build/jclsh/genapp-csd/catalog.dat --csd-list GENALIST --dataset-profile extras/jclsh/tests/genapp-vsam.conf --program LGTESTC1=unused-worker --transid SSC1 --check-csd
    if ($LASTEXITCODE -ne 0 -or $csdCheck -ne 'CSD routes=1 files=2') {
        throw "cobcicsd did not resolve GenApp CSD and VSAM: $csdCheck"
    }
    $indexRoot = Join-Path $workspace 'build/jclsh/carddemo-index'
    New-Item -ItemType Directory -Force $indexRoot | Out-Null
    $indexRecords = @(
        (New-TestRecord '0000000000000002' '2022-06-20'),
        (New-TestRecord '0000000000000001' '2022-05-20')
    )
    [System.IO.File]::WriteAllText((Join-Path $indexRoot 'base.dat'), (($indexRecords -join "`n") + "`n"), [System.Text.Encoding]::ASCII)
    foreach ($name in @('index.dat', 'path.dat')) {
        $path = Join-Path $indexRoot $name
        if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path }
    }
    cargo run --locked --manifest-path extras/jclsh/Cargo.toml -- run reference/aws-mainframe-modernization-carddemo/app/jcl/TRANIDX.jcl --profile extras/jclsh/tests/carddemo-index.conf
    if ($LASTEXITCODE -ne 0) { throw 'original CardDemo AIX JCL failed' }
    $indexLines = [System.IO.File]::ReadAllLines((Join-Path $indexRoot 'index.dat'))
    if ($indexLines.Length -ne 3 -or $indexLines[1] -notmatch "`t1$" -or $indexLines[2] -notmatch "`t0$") {
        throw 'CardDemo BLDINDEX order differs'
    }
    if ((Get-Content -LiteralPath (Join-Path $indexRoot 'path.dat') -Raw) -notmatch '^#JCLSH-PATH') {
        throw 'CardDemo PATH definition missing'
    }
    $gdgRoot = Join-Path $workspace 'build/jclsh/carddemo-gdg'
    New-Item -ItemType Directory -Force $gdgRoot | Out-Null
    Get-ChildItem -LiteralPath $gdgRoot -File | Remove-Item
    cargo run --locked --manifest-path extras/jclsh/Cargo.toml -- run reference/aws-mainframe-modernization-carddemo/app/jcl/DEFGDGB.jcl --profile extras/jclsh/tests/carddemo-gdg.conf
    if ($LASTEXITCODE -ne 0) { throw 'original CardDemo GDG JCL failed' }
    if ((Get-ChildItem -LiteralPath $gdgRoot -File).Count -ne 6) { throw 'CardDemo GDG definitions missing' }
    cargo run --locked --manifest-path extras/jclsh/Cargo.toml -- run reference/aws-mainframe-modernization-carddemo/app/jcl/DEFGDGB.jcl --profile extras/jclsh/tests/carddemo-gdg.conf
    if ($LASTEXITCODE -ne 0) { throw 'CardDemo duplicate GDG handling failed' }
    $gdgDataRoot = Join-Path $workspace 'build/jclsh/carddemo-gdg-data'
    New-Item -ItemType Directory -Force $gdgDataRoot | Out-Null
    Get-ChildItem -LiteralPath $gdgDataRoot -File | Remove-Item
    foreach ($sourceName in @('trantype', 'trancatg', 'discgrp')) {
        [System.IO.File]::WriteAllText((Join-Path $gdgDataRoot "$sourceName.dat"), "$sourceName`n")
    }
    cargo run --locked --manifest-path extras/jclsh/Cargo.toml -- run reference/aws-mainframe-modernization-carddemo/app/jcl/DEFGDGD.jcl --profile extras/jclsh/tests/carddemo-gdg-data.conf
    if ($LASTEXITCODE -ne 0) { throw 'original CardDemo GDG data JCL failed' }
    foreach ($sourceName in @('trantype', 'trancatg', 'discgrp')) {
        $generation = [System.IO.File]::ReadAllBytes((Join-Path $gdgDataRoot "$sourceName-backup.dat.G0001V00"))
        $expectedLength = if ($sourceName -eq 'discgrp') { 50 } else { 60 }
        if ($generation.Length -ne $expectedLength -or
            [System.Text.Encoding]::UTF8.GetString($generation, 0, $sourceName.Length) -ne $sourceName) {
            throw "CardDemo GDG generation differs: $sourceName"
        }
    }
    $generationRoot = Join-Path $workspace 'build/jclsh/gdg-generations'
    New-Item -ItemType Directory -Force $generationRoot | Out-Null
    Get-ChildItem -LiteralPath $generationRoot -File | Remove-Item
    [System.IO.File]::WriteAllText((Join-Path $generationRoot 'source.dat'), "generation`n")
    cargo run --locked --manifest-path extras/jclsh/Cargo.toml -- run extras/jclsh/tests/gdg-generations.jcl --profile extras/jclsh/tests/gdg-generations.conf
    if ($LASTEXITCODE -ne 0) { throw 'GDG generation JCL failed' }
    if (Test-Path -LiteralPath (Join-Path $generationRoot 'base.dat.G0001V00')) { throw 'GDG LIMIT did not scratch oldest generation' }
    if (-not (Test-Path -LiteralPath (Join-Path $generationRoot 'base.dat.G0002V00')) -or
        -not (Test-Path -LiteralPath (Join-Path $generationRoot 'base.dat.G0003V00'))) { throw 'GDG generations missing' }
    if ((Get-Content -LiteralPath (Join-Path $generationRoot 'same.dat') -Raw) -ne "generation`n" -or
        (Get-Content -LiteralPath (Join-Path $generationRoot 'latest.dat') -Raw) -ne "generation`n" -or
        (Get-Content -LiteralPath (Join-Path $generationRoot 'previous.dat') -Raw) -ne "generation`n") { throw 'GDG relative generation read differs' }
    $combineRoot = Join-Path $workspace 'build/jclsh/carddemo-combtran'
    New-Item -ItemType Directory -Force $combineRoot | Out-Null
    Get-ChildItem -LiteralPath $combineRoot -File | Remove-Item
    foreach ($baseName in @('transaction-backup', 'system-transactions', 'combined')) {
        [System.IO.File]::WriteAllText((Join-Path $combineRoot "$baseName.dat"), "#JCLSH-GDG`t5`n")
    }
    function New-CombinedRecord([string]$id) {
        $chars = [char[]](' ' * 350)
        $id.CopyTo(0, $chars, 0, 16)
        return [string]::new($chars)
    }
    [System.IO.File]::WriteAllText((Join-Path $combineRoot 'transaction-backup.dat.G0001V00'), ((New-CombinedRecord '0000000000000002') + "`n"), [System.Text.Encoding]::ASCII)
    [System.IO.File]::WriteAllText((Join-Path $combineRoot 'system-transactions.dat.G0001V00'), ((New-CombinedRecord '0000000000000001') + "`n"), [System.Text.Encoding]::ASCII)
    [System.IO.File]::WriteAllText((Join-Path $combineRoot 'transaction-vsam.dat'), '')
    cargo run --locked --manifest-path extras/jclsh/Cargo.toml -- run reference/aws-mainframe-modernization-carddemo/app/jcl/COMBTRAN.jcl --profile extras/jclsh/tests/carddemo-combtran.conf
    if ($LASTEXITCODE -ne 0) { throw 'original CardDemo COMBTRAN JCL failed' }
    $combinedRecords = [System.IO.File]::ReadAllLines((Join-Path $combineRoot 'transaction-vsam.dat'))
    if ($combinedRecords.Length -ne 2 -or $combinedRecords[0].Substring(0, 16) -ne '0000000000000001' -or
        $combinedRecords[1].Substring(0, 16) -ne '0000000000000002') {
        throw 'COMBTRAN concatenation, SORT or GDG REPRO differs'
    }
    $cardCsdRoot = Join-Path $workspace 'build/jclsh/carddemo-csd'
    New-Item -ItemType Directory -Force $cardCsdRoot | Out-Null
    [System.IO.File]::WriteAllText((Join-Path $cardCsdRoot 'catalog.dat'), '')
    cargo run --locked --manifest-path extras/jclsh/Cargo.toml -- run reference/aws-mainframe-modernization-carddemo/app/jcl/CBADMCDJ.jcl --profile extras/jclsh/tests/carddemo-csd.conf
    if ($LASTEXITCODE -ne 0) { throw 'original CardDemo DFHCSDUP JCL failed' }
    $cardCatalog = Get-Content -LiteralPath (Join-Path $cardCsdRoot 'catalog.dat')
    if (-not ($cardCatalog | Where-Object { $_ -match '^R\s+CARDDEMO\s+TRANSACTION\s+CCDM\s+' })) {
        throw 'CardDemo CSD transaction CCDM missing'
    }
    $unsupported = & extras/jclsh/target/debug/jclsh.exe run extras/jclsh/tests/unsupported-utility.jcl --profile extras/jclsh/tests/utilities.conf
    if ($LASTEXITCODE -eq 0 -or -not ($unsupported | Where-Object { $_ -match 'JCLSH STEP ALTER RC=12' })) {
        throw 'unsupported IDCAMS command did not return RC 12'
    }
    Write-Output 'jclsh utility JCL passed: IEBGENER, SORT, IDCAMS, IEFBR14, DFHCSDUP, CardDemo and GenApp originals'
} finally {
    Pop-Location
}
