$ErrorActionPreference = 'Stop'
$bundleRoot = Join-Path $PSScriptRoot '..\test-results\verifier-bundle'
New-Item -ItemType Directory -Path $bundleRoot -Force | Out-Null
$pythonZip = Join-Path $bundleRoot 'python.zip'
Invoke-WebRequest 'https://www.python.org/ftp/python/3.11.9/python-3.11.9-embed-amd64.zip' -OutFile $pythonZip
$runtimePath = Join-Path $bundleRoot 'content\python'
Expand-Archive -LiteralPath $pythonZip -DestinationPath $runtimePath -Force
Set-Content -LiteralPath (Join-Path $runtimePath 'python311._pth') -Value "python311.zip`n.`n..\libraries" -Encoding ascii
$uefiZip = Join-Path $bundleRoot 'uefi.zip'
Invoke-WebRequest 'https://github.com/LongSoft/UEFITool/releases/download/A75/UEFIExtract_NE_A75_win64.zip' -OutFile $uefiZip
if ((Get-FileHash $uefiZip).Hash.ToLowerInvariant() -ne '20ff18208913d32c99e3b002717abeddaa3b6509ac62e6699e462b0f533be646') { throw 'UEFIExtract release hash mismatch' }
Expand-Archive -LiteralPath $uefiZip -DestinationPath (Join-Path $bundleRoot 'uefi') -Force
Copy-Item -LiteralPath (Join-Path $bundleRoot 'uefi\UEFIExtract.exe') -Destination (Join-Path $bundleRoot 'content\UEFIExtract.exe')
$mcePath = Join-Path $bundleRoot 'content\MCE'
New-Item -ItemType Directory -Path $mcePath -Force | Out-Null
foreach ($name in @('MCE.py','MCE.db','LICENSE','README.md')) {
  Invoke-WebRequest "https://raw.githubusercontent.com/platomav/MCExtractor/r352/$name" -OutFile (Join-Path $mcePath $name)
}
Invoke-WebRequest 'https://raw.githubusercontent.com/LongSoft/UEFITool/A75/LICENSE.md' -OutFile (Join-Path $bundleRoot 'content\UEFITool-LICENSE.md')
$libraryPath = Join-Path $bundleRoot 'content\libraries'
New-Item -ItemType Directory -Path $libraryPath -Force | Out-Null
python -m pip install --disable-pip-version-check --no-compile --no-deps --target $libraryPath colorama==0.4.6 pltable==1.1.0
if ($LASTEXITCODE -ne 0) { throw 'Verifier dependencies could not be bundled' }
Add-Type -AssemblyName System.IO.Compression.FileSystem
$archivePath = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\assets\verification-bundle.zip'))
if (Test-Path -LiteralPath $archivePath) { Remove-Item -LiteralPath $archivePath }
[IO.Compression.ZipFile]::CreateFromDirectory((Join-Path $bundleRoot 'content'), $archivePath)
Get-FileHash $archivePath
