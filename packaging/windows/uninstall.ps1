# Takes out what install.ps1 added: the Start Menu shortcut, %LOCALAPPDATA%\cloofy\bin, and that
# one PATH entry where install.ps1 put it. Your configuration, cache and sign-ins stay.
$ErrorActionPreference = 'Stop'

$bin = Join-Path $env:LOCALAPPDATA 'cloofy\bin'
$exe = Join-Path $bin 'cloofy.exe'

Write-Host "`n  cloofy for Windows`n"

if (Test-Path $exe) {
    & $exe windows remove
}

# Only the entry install.ps1 adds, and only where it is; the rest of the PATH stays as it is.
$path = [Environment]::GetEnvironmentVariable('PATH', 'User')
if ($path) {
    $entries = $path -split ';' | Where-Object { $_ -and $_ -ne $bin }
    if ($entries.Count -ne ($path -split ';' | Where-Object { $_ }).Count) {
        [Environment]::SetEnvironmentVariable('PATH', ($entries -join ';'), 'User')
        Write-Host "  $bin is off your PATH"
    }
}

if (Test-Path $bin) {
    Remove-Item $bin -Recurse -Force
    Write-Host "  removed $bin"
}

Write-Host "`n  Done. Your configuration, cache and sign-ins stay in %APPDATA% and %LOCALAPPDATA%\cloofy.`n"
