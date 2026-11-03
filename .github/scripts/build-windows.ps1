param([switch] $DebugBuild)
$ErrorActionPreference = 'Stop'
. "$PSScriptRoot/windows-dependencies.ps1"

$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
$visualStudio = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if ($LASTEXITCODE -ne 0 -or -not $visualStudio) { throw 'Visual C++ Build Tools are required' }
$runtimeVersion = (Get-Content -Raw (Join-Path $visualStudio 'VC/Auxiliary/Build/Microsoft.VCRedistVersion.default.txt')).Trim()
$runtime = Join-Path $visualStudio "VC/Redist/MSVC/$runtimeVersion/x64/Microsoft.VC143.CRT"
$resources = @{}
foreach ($name in @('msvcp140.dll', 'vcruntime140.dll', 'vcruntime140_1.dll', 'concrt140.dll')) {
    $path = Join-Path $runtime $name
    if (-not (Test-Path -LiteralPath $path)) { throw "Missing runtime: $name" }
    $resources[$path] = $name
}
$resources[(Join-Path $opencvRoot 'x64/vc16/bin/opencv_world4120.dll')] = 'opencv_world4120.dll'
$resources[(Join-Path $opencvRoot 'LICENSE')] = 'licenses/opencv/LICENSE.txt'
$resources[(Join-Path $opencvRoot 'etc/licenses/')] = 'licenses/opencv/third-party/'
$config = Join-Path $nativeRoot 'windows-bundle.json'
@{ bundle = @{ resources = $resources } } | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $config -Encoding UTF8

Push-Location (Join-Path $repoRoot 'desktop')
try {
    $arguments = @('build', '--ci', '--config', $config, '--target', 'x86_64-pc-windows-msvc')
    if ($DebugBuild) { $arguments += '--debug' }
    & (Join-Path $repoRoot 'frontend/node_modules/.bin/tauri.cmd') @arguments -- --locked
    if ($LASTEXITCODE -ne 0) { throw 'Windows installer build failed' }
} finally { Pop-Location }

$profile = if ($DebugBuild) { 'debug' } else { 'release' }
$installers = @(Get-ChildItem -LiteralPath (Join-Path $repoRoot "target/x86_64-pc-windows-msvc/$profile/bundle/nsis") -Filter '*-setup.exe' -File)
if ($installers.Count -ne 1) { throw 'Expected exactly one NSIS installer; remove stale build outputs manually' }
node (Join-Path $repoRoot '.github/scripts/release.mjs') stage $installers[0].FullName $profile
if ($LASTEXITCODE -ne 0) { throw 'Installer manifest failed' }
