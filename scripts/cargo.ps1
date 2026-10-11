param([Parameter(ValueFromRemainingArguments = $true)][string[]]$CargoArgs)
$projectRoot = Split-Path -Parent $PSScriptRoot
$env:CARGO_HOME = Join-Path $projectRoot '.tools\cargo'
$env:RUSTUP_HOME = Join-Path $projectRoot '.tools\rustup'
$env:Path = "$env:CARGO_HOME\bin;$env:Path"
& "$env:CARGO_HOME\bin\cargo.exe" @CargoArgs
exit $LASTEXITCODE
