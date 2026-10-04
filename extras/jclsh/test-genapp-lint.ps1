$ErrorActionPreference = 'Stop'
$workspace = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$binary = Join-Path $workspace 'extras/jclsh/target/debug/jclsh.exe'
Push-Location $workspace
try {
    cargo build --manifest-path extras/jclsh/Cargo.toml --offline | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'jclsh build failed' }
    $passed = 0
    $externalUtilityProc = 0
    $files = @(Get-ChildItem -LiteralPath 'reference/cics-genapp/base/cntl' -File -Filter '*.jcl')
    foreach ($file in $files) {
        $ErrorActionPreference = 'Continue'
        $output = & $binary lint $file.FullName 2>&1
        $ErrorActionPreference = 'Stop'
        if ($LASTEXITCODE -eq 0) {
            $passed++
        } elseif (($output -join ' ') -match 'member (DFHLS2WS|EYUJXBTP) not found in JCLLIB ORDER') {
            $externalUtilityProc++
        } else {
            throw "unexpected JCL error in $($file.Name): $($output -join ' ')"
        }
    }
    if ($files.Count -ne 29 -or $passed -ne 20 -or $externalUtilityProc -ne 9) {
        throw "unexpected JCL inventory: total=$($files.Count) parsed=$passed externalUtilityProc=$externalUtilityProc"
    }
    Write-Output "GenApp JCL passed: $passed parsed; $externalUtilityProc require external IBM utility PROC members"
}
finally {
    Pop-Location
}
