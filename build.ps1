param([switch]$SkipPublish)
$ErrorActionPreference = 'Stop'
if (!(Test-Path -LiteralPath (Join-Path $PSScriptRoot 'assets\RochMicrocode.ico'))) { & (Join-Path $PSScriptRoot 'tools\Generate-Icon.ps1') }
$projectPath = Join-Path $PSScriptRoot 'src\RochBios.App\RochBios.App.csproj'
dotnet build $projectPath -c Release --nologo
if ($LASTEXITCODE -ne 0) { throw 'Build failed' }
if (!$SkipPublish) {
    dotnet publish $projectPath -c Release -o (Join-Path $PSScriptRoot 'dist') --nologo -p:DebugType=None -p:DebugSymbols=false
    if ($LASTEXITCODE -ne 0) { throw 'Publish failed' }
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'README.md') -Destination (Join-Path $PSScriptRoot 'dist\README.md')
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'LICENSE') -Destination (Join-Path $PSScriptRoot 'dist\LICENSE')
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'CHANGELOG.md') -Destination (Join-Path $PSScriptRoot 'dist\CHANGELOG.md')
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'VALIDATION.md') -Destination (Join-Path $PSScriptRoot 'dist\VALIDATION.md')
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'THIRD-PARTY.md') -Destination (Join-Path $PSScriptRoot 'dist\THIRD-PARTY.md')
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'THIRD-PARTY') -Destination (Join-Path $PSScriptRoot 'dist') -Recurse -Force
}
