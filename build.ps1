# Builds the portable Windows x64 app with Cargo and assembles dist/.
# Requires the Rust toolchain (stable, x86_64-pc-windows-msvc) and the MSVC build tools.
param([switch]$SkipPublish)
$ErrorActionPreference = 'Stop'
Push-Location $PSScriptRoot
try {
    if (!(Test-Path -LiteralPath 'assets\RochMicrocode.ico')) { & (Join-Path $PSScriptRoot 'tools\Generate-Icon.ps1') }
    cargo build --release --locked -p roch-app
    if ($LASTEXITCODE -ne 0) { throw 'Build failed' }
    if (!$SkipPublish) {
        $dist = Join-Path $PSScriptRoot 'dist'
        New-Item -ItemType Directory -Path $dist -Force | Out-Null
        Copy-Item -LiteralPath 'target\release\RochMicrocode.exe' -Destination (Join-Path $dist 'RochMicrocode.exe') -Force
        foreach ($name in @('README.md', 'LICENSE', 'CHANGELOG.md', 'VALIDATION.md', 'THIRD-PARTY.md')) {
            Copy-Item -LiteralPath $name -Destination (Join-Path $dist $name) -Force
        }
        Copy-Item -LiteralPath 'THIRD-PARTY' -Destination $dist -Recurse -Force
    }
}
finally { Pop-Location }
