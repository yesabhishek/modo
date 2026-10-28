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
