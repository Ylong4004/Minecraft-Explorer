param([string]$Python = 'python', [string]$Executable = '', [string]$OutputDirectory = '')
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$distribution = if ($OutputDirectory) { [System.IO.Path]::GetFullPath($OutputDirectory) } else { Join-Path $projectRoot 'dist\MinecraftExplorer' }
New-Item -ItemType Directory -Path $distribution -Force | Out-Null
$env:CARGO_HOME = Join-Path $projectRoot '.tools\cargo'
$env:RUSTUP_HOME = Join-Path $projectRoot '.tools\rustup'
$localCargo = Join-Path $env:CARGO_HOME 'bin\cargo.exe'
if (!(Test-Path -LiteralPath $localCargo)) { $localCargo = 'cargo' }
$metadataPath = Join-Path $projectRoot '.test-artifacts\package-metadata.json'
New-Item -ItemType Directory -Path (Split-Path -Parent $metadataPath) -Force | Out-Null
& $localCargo metadata --locked --format-version 1 --filter-platform x86_64-pc-windows-msvc --manifest-path (Join-Path $projectRoot 'Cargo.toml') | Set-Content -LiteralPath $metadataPath -Encoding UTF8
if ($LASTEXITCODE -ne 0) { throw 'Cargo metadata failed' }
$packageVersion = ((Get-Content -LiteralPath $metadataPath -Raw -Encoding UTF8 | ConvertFrom-Json).packages | Where-Object { $_.name -eq 'minecraft-explorer' }).version
if (!$Executable) { $Executable = Join-Path $projectRoot 'target\release\minecraft-explorer.exe' }
$artifact = Get-Item -LiteralPath $Executable
if ($artifact.VersionInfo.ProductVersion -ne $packageVersion) {
    throw "发行 EXE 版本与 Cargo 包版本不一致：$($artifact.VersionInfo.ProductVersion) / $packageVersion。请先构建当前普通 Release，或选择对应的已验收 EXE。"
}
Copy-Item -LiteralPath $Executable -Destination (Join-Path $distribution 'MinecraftExplorer.exe') -Force
foreach ($name in @('README.md','LICENSE')) {
    Copy-Item -LiteralPath (Join-Path $projectRoot $name) -Destination $distribution -Force
}
$distributionDocs = Join-Path $distribution 'docs'
New-Item -ItemType Directory -Path $distributionDocs -Force | Out-Null
foreach ($document in (Get-ChildItem -LiteralPath (Join-Path $projectRoot 'docs') -File)) {
    $legacyDocument = Join-Path $distribution $document.Name
    if (Test-Path -LiteralPath $legacyDocument -PathType Leaf) {
        Move-Item -LiteralPath $legacyDocument -Destination (Join-Path $distributionDocs $document.Name) -Force
    }
    Copy-Item -LiteralPath $document.FullName -Destination $distributionDocs -Force
}
& $Python (Join-Path $PSScriptRoot 'collect_licenses.py') $metadataPath $distribution
if ($LASTEXITCODE -ne 0) { throw 'License collection failed' }
$hash = (Get-FileHash -LiteralPath (Join-Path $distribution 'MinecraftExplorer.exe') -Algorithm SHA256).Hash
Set-Content -LiteralPath (Join-Path $distribution 'SHA256.txt') -Value "$hash  MinecraftExplorer.exe" -Encoding ASCII
$archive = Join-Path $projectRoot "dist\MinecraftExplorer-$packageVersion-win-x64.zip"
& $Python (Join-Path $PSScriptRoot 'create_archive.py') $distribution $archive
if ($LASTEXITCODE -ne 0) { throw 'Archive verification failed' }
# 同步 1.0 等发行系列的单文件入口，避免 dist 根目录保留同名旧 EXE。
$releaseParts = $packageVersion.Split('.')
$singleFileName = "MinecraftExplorer-$($releaseParts[0]).$($releaseParts[1])-win-x64.exe"
$singleFile = Join-Path $projectRoot "dist\$singleFileName"
if (!(Test-Path -LiteralPath $singleFile) -or (Get-FileHash -LiteralPath $singleFile -Algorithm SHA256).Hash -ne $hash) {
    Copy-Item -LiteralPath $Executable -Destination $singleFile -Force
}
Write-Output $distribution
Write-Output $singleFile
Write-Output $archive
Write-Output "SHA256: $hash"
