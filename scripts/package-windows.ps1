# Build the Windows installer and a portable ZIP into dist\.
#   pwsh scripts/package-windows.ps1
# Requires NSIS 3 (makensis on PATH). Optional Authenticode signing:
#   $env:WINDOWS_CERTIFICATE_BASE64, $env:WINDOWS_CERTIFICATE_PASSWORD
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path -Parent $PSScriptRoot)

$Version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
$Numeric = ($Version -split '[-+]')[0]
cargo build --locked --release
if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }

$Stem = "modo-$Version-windows-x64"
$Staging = Join-Path ([IO.Path]::GetTempPath()) $Stem
Remove-Item -Recurse -Force $Staging -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $Staging, dist | Out-Null
Copy-Item target\release\modo.exe, assets\package\modo.ico, LICENSE $Staging
python scripts/third-party-notices.py --target x86_64-pc-windows-msvc | Out-Null
Copy-Item THIRD_PARTY_NOTICES.md $Staging

# Windows Terminal's ConPTY is faster and more correct than the console host
# built into Windows. The terminal loads conpty.dll from beside modo.exe, and
# conpty.dll starts the OpenConsole.exe next to it.
$ConptyVersion = '1.24.261001001'
$ConptySha256 = '4d6aaddc1d2385c9f5897df28f33879f699f8f2783315d5204cf3d8c3616ac5f'
$ConptyPackage = Join-Path ([IO.Path]::GetTempPath()) "conpty-$ConptyVersion.nupkg"
Invoke-WebRequest -UseBasicParsing -OutFile $ConptyPackage `
    -Uri "https://api.nuget.org/v3-flatcontainer/microsoft.windows.console.conpty/$ConptyVersion/microsoft.windows.console.conpty.$ConptyVersion.nupkg"
if ((Get-FileHash -Algorithm SHA256 $ConptyPackage).Hash.ToLower() -ne $ConptySha256) {
    throw 'The ConPTY package does not match its pinned checksum'
}
Add-Type -AssemblyName System.IO.Compression.FileSystem
$Archive = [IO.Compression.ZipFile]::OpenRead($ConptyPackage)
try {
    foreach ($Entry in 'runtimes/win-x64/native/conpty.dll', 'build/native/runtimes/x64/OpenConsole.exe') {
        $File = $Archive.GetEntry($Entry)
        if (-not $File) { throw "The ConPTY package has no $Entry" }
        [IO.Compression.ZipFileExtensions]::ExtractToFile($File, (Join-Path $Staging (Split-Path $Entry -Leaf)), $true)
    }
} finally {
    $Archive.Dispose()
}
# Single quotes: in a double-quoted string PowerShell would read the fences as escapes.
$ConptyNotice = @'

## Windows Terminal ConPTY VERSION (conpty.dll, OpenConsole.exe)

License: MIT
Source: https://github.com/microsoft/terminal

````
MIT License

Copyright (c) Microsoft Corporation. All rights reserved.

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
````
'@
Add-Content (Join-Path $Staging 'THIRD_PARTY_NOTICES.md') $ConptyNotice.Replace('VERSION', $ConptyVersion)

function Sign([string]$Path) {
    if (-not $env:WINDOWS_CERTIFICATE_BASE64) { return }
    $Pfx = Join-Path $Staging 'signing.pfx'
    [IO.File]::WriteAllBytes($Pfx, [Convert]::FromBase64String($env:WINDOWS_CERTIFICATE_BASE64))
    $SignTool = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\signtool.exe" |
        Sort-Object FullName -Descending | Select-Object -First 1 -ExpandProperty FullName
    & $SignTool sign /f $Pfx /p $env:WINDOWS_CERTIFICATE_PASSWORD /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 $Path
    if ($LASTEXITCODE -ne 0) { throw "Signing $Path failed" }
    Remove-Item $Pfx
}

Sign (Join-Path $Staging 'modo.exe')
$Installer = Join-Path (Resolve-Path dist) "$Stem-setup.exe"
makensis /V2 "/DVERSION=$Version" "/DNUMERIC_VERSION=$Numeric" "/DSTAGING=$Staging" "/DOUTFILE=$Installer" assets\package\windows.nsi
if ($LASTEXITCODE -ne 0) { throw 'makensis failed' }
Sign $Installer

$Zip = "dist\$Stem.zip"
Compress-Archive -Force -Path "$Staging\*" -DestinationPath $Zip
Get-FileHash -Algorithm SHA256 $Installer, $Zip |
    ForEach-Object { "$($_.Hash.ToLower())  $(Split-Path $_.Path -Leaf)" } |
    Set-Content "dist\$Stem-SHA256SUMS.txt"
Write-Host "Built $Installer"
