# Builds cloofy for Windows, puts it in %LOCALAPPDATA%\cloofy\bin and adds the Start Menu
# shortcut Windows shows its notifications under.
#
#   .\packaging\windows\install.ps1                      YouTube Music
#   .\packaging\windows\install.ps1 -Service spotify     Spotify
#   .\packaging\windows\install.ps1 -Service both        both
#   .\packaging\windows\install.ps1 -AddToPath           and on your PATH, to run `cloofy`
param(
    [ValidateSet('ytmusic', 'spotify', 'both')]
    [string]$Service = 'ytmusic',
    [switch]$AddToPath
)

$ErrorActionPreference = 'Stop'

$repo = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$features = if ($Service -eq 'both') { 'spotify,ytmusic' } else { $Service }
$bin = Join-Path $env:LOCALAPPDATA 'cloofy\bin'
$exe = Join-Path $bin 'cloofy.exe'

Write-Host "`n  cloofy for Windows: $Service build`n"

Push-Location $repo
try {
    cargo build --release --locked -p cloofy --no-default-features --features $features
} finally {
    Pop-Location
}

New-Item -ItemType Directory -Force -Path $bin | Out-Null
Copy-Item (Join-Path $repo 'target\release\cloofy.exe') $exe -Force
Write-Host "  cloofy is in $bin"

& $exe windows install

if ($AddToPath) {
    $path = [Environment]::GetEnvironmentVariable('PATH', 'User')
    if (($path -split ';') -notcontains $bin) {
        $path = if ([string]::IsNullOrEmpty($path)) { $bin } else { $path.TrimEnd(';') + ';' + $bin }
        [Environment]::SetEnvironmentVariable('PATH', $path, 'User')
        Write-Host "  $bin is on your PATH; open a new terminal to run cloofy"
    }
}

Write-Host "`n  Done. Run cloofy login to set it up, then cloofy.`n"
