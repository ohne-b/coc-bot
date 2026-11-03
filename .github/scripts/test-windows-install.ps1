param([Parameter(Mandatory)][string] $Installer)
$ErrorActionPreference = 'Stop'
$installerPath = (Resolve-Path -LiteralPath $Installer).Path
$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$installRoot = [IO.Path]::GetFullPath((Join-Path $repoRoot ('.tools/install-check-' + [guid]::NewGuid().ToString('N'))))
if (-not $installRoot.StartsWith((Join-Path $repoRoot '.tools\'), [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Installation check must stay inside repository tooling storage'
}
$uninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\CoC Bot'
$productKey = 'HKCU:\Software\ohne-b\CoC Bot'
foreach ($key in @($uninstallKey, $productKey, 'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\CoC Bot', 'HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\CoC Bot')) {
    if (Test-Path -LiteralPath $key) { throw 'An existing CoC Bot installation or installer preference would be affected' }
}
if (Get-Process -Name 'coc-bot-desktop' -ErrorAction SilentlyContinue) { throw 'Close CoC Bot before testing its installer' }
$autorun = Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -ErrorAction SilentlyContinue
if ($autorun -and $autorun.PSObject.Properties['CoC Bot']) { throw 'An existing CoC Bot startup preference would be affected' }

function Invoke-CheckedProcess([string] $Program, [string] $Arguments, [int] $Expected = 0) {
    $process = Start-Process -FilePath $Program -ArgumentList $Arguments -WindowStyle Hidden -PassThru
    try {
        if (-not $process.WaitForExit(120000)) {
            $process.Kill()
            $process.WaitForExit()
            throw "Installation check timed out: $Program"
        }
        if ($process.ExitCode -ne $Expected) { throw "Unexpected exit code $($process.ExitCode), expected ${Expected}: $Program" }
    } finally { $process.Dispose() }
}

$previousPath = $env:PATH
try {
    Invoke-CheckedProcess $installerPath "/S /NS /D=$installRoot"
    $exe = Join-Path $installRoot 'coc-bot-desktop.exe'
    foreach ($file in @('coc-bot-desktop.exe', 'uninstall.exe', 'config.json', 'opencv_world4120.dll', 'msvcp140.dll', 'vcruntime140.dll', 'vcruntime140_1.dll', 'concrt140.dll', 'LICENSE.md', 'NOTICE.md', 'licenses/Manrope-OFL.txt', 'licenses/Material-Design-Icons.txt', 'licenses/opencv/LICENSE.txt')) {
        if (-not (Test-Path -LiteralPath (Join-Path $installRoot $file) -PathType Leaf)) { throw "Missing installed file: $file" }
    }
    if (Get-ChildItem -LiteralPath $installRoot -Recurse -File | Where-Object { $_.Extension -in @('.py', '.pyc') -or $_.Name -match '^python.*\.(exe|dll)$' }) { throw 'Python must not be bundled' }
    $env:PATH = "$env:SystemRoot\System32;$env:SystemRoot"
    Invoke-CheckedProcess $exe '--check-installation'
    $config = Join-Path $installRoot 'config.json'
    $backup = Join-Path $installRoot 'config.check-backup'
    Move-Item -LiteralPath $config -Destination $backup
    try { Invoke-CheckedProcess $exe '--check-installation' 1 }
    finally { Move-Item -LiteralPath $backup -Destination $config }
    Write-Output 'Installed calibration, OpenCV and SQLite passed without development PATH entries.'
} finally {
    $env:PATH = $previousPath
    $uninstaller = Join-Path $installRoot 'uninstall.exe'
    if (Test-Path -LiteralPath $uninstaller) {
        Invoke-CheckedProcess $uninstaller "/S _?=$installRoot"
        if (Test-Path -LiteralPath $uninstaller) { Remove-Item -LiteralPath $uninstaller }
    }
    if (Test-Path -LiteralPath $installRoot) {
        if (Get-ChildItem -LiteralPath $installRoot -Force) { throw "Installer left files in $installRoot" }
        Remove-Item -LiteralPath $installRoot
    }
    if (Test-Path -LiteralPath $uninstallKey) { throw 'Uninstaller left its registration behind' }
    if (Test-Path -LiteralPath $productKey) { Remove-Item -LiteralPath $productKey }
}
Write-Output 'Installation, missing-asset failure and uninstallation passed. Personal app data was not opened.'
