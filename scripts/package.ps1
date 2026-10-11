param([string]$Python = 'python', [string]$Executable = '')
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$distribution = Join-Path $projectRoot 'dist\MinecraftExplorer'
New-Item -ItemType Directory -Path $distribution -Force | Out-Null
$env:CARGO_HOME = Join-Path $projectRoot '.tools\cargo'
$env:RUSTUP_HOME = Join-Path $projectRoot '.tools\rustup'
$localCargo = Join-Path $env:CARGO_HOME 'bin\cargo.exe'
if (!(Test-Path -LiteralPath $localCargo)) { $localCargo = 'cargo' }
$metadataPath = Join-Path $projectRoot '.test-artifacts\package-metadata.json'
New-Item -ItemType Directory -Path (Split-Path -Parent $metadataPath) -Force | Out-Null
& $localCargo metadata --locked --format-version 1 --filter-platform x86_64-pc-windows-msvc --manifest-path (Join-Path $projectRoot 'Cargo.toml') | Set-Content -LiteralPath $metadataPath -Encoding UTF8
if ($LASTEXITCODE -ne 0) { throw 'Cargo metadata failed' }
if (!$Executable) { $Executable = Join-Path $projectRoot 'target\release\minecraft-explorer.exe' }
Copy-Item -LiteralPath $Executable -Destination (Join-Path $distribution 'MinecraftExplorer.exe') -Force
foreach ($name in @('README.md','ARCHITECTURE.md','TESTING.md','LAUNCHER_RESEARCH.md','DEVELOPMENT.md','THIRD_PARTY.md','LICENSE')) {
    Copy-Item -LiteralPath (Join-Path $projectRoot $name) -Destination $distribution -Force
}
& $Python (Join-Path $PSScriptRoot 'collect_licenses.py') $metadataPath $distribution
if ($LASTEXITCODE -ne 0) { throw 'License collection failed' }
$hash = (Get-FileHash -LiteralPath (Join-Path $distribution 'MinecraftExplorer.exe') -Algorithm SHA256).Hash
Set-Content -LiteralPath (Join-Path $distribution 'SHA256.txt') -Value "$hash  MinecraftExplorer.exe" -Encoding ASCII
$packageVersion = ((Get-Content -LiteralPath $metadataPath -Raw -Encoding UTF8 | ConvertFrom-Json).packages | Where-Object { $_.name -eq 'minecraft-explorer' }).version
$archive = Join-Path $projectRoot "dist\MinecraftExplorer-$packageVersion-win-x64.zip"
& $Python (Join-Path $PSScriptRoot 'create_archive.py') $distribution $archive
if ($LASTEXITCODE -ne 0) { throw 'Archive verification failed' }
Write-Output $distribution
Write-Output $archive
Write-Output "SHA256: $hash"
