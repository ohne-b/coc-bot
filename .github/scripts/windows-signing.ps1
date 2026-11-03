$ErrorActionPreference = 'Stop'
$signingRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../.tools'))
New-Item -ItemType Directory -Path $signingRoot -Force | Out-Null
$archive = Join-Path $signingRoot 'minisign-0.12-win64.zip'
if (-not (Test-Path -LiteralPath $archive)) {
    Invoke-WebRequest 'https://github.com/jedisct1/minisign/releases/download/0.12/minisign-0.12-win64.zip' -OutFile $archive
}
if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash -ne '37b600344e20c19314b2e82813db2bfdcc408b77b876f7727889dbd46d539479') {
    throw 'Minisign archive checksum mismatch'
}
Expand-Archive -LiteralPath $archive -DestinationPath (Join-Path $signingRoot 'minisign-0.12') -Force
