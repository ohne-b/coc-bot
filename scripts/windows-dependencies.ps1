$ErrorActionPreference = 'Stop'

# Dot-source this file before Cargo commands so native DLLs and headers remain available.
$repoRoot = Split-Path $PSScriptRoot -Parent
$nativeRoot = Join-Path $repoRoot '.tools'
New-Item -ItemType Directory -Path $nativeRoot -Force | Out-Null

function Get-VerifiedArchive([string] $Name, [string] $Url, [string] $Sha256) {
    $archive = Join-Path $nativeRoot $Name
    if (-not (Test-Path -LiteralPath $archive)) {
        Invoke-WebRequest -Uri $Url -OutFile $archive
    }
    if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash -ne $Sha256) {
        throw "Checksum mismatch for $Name. Remove the corrupt archive and retry."
    }
    return $archive
}

$opencvRoot = Join-Path $nativeRoot 'opencv\build'
if (-not (Test-Path -LiteralPath "$opencvRoot\x64\vc16\bin\opencv_world4120.dll")) {
    $archive = Get-VerifiedArchive 'opencv-4.12.0-windows.exe' `
        'https://github.com/opencv/opencv/releases/download/4.12.0/opencv-4.12.0-windows.exe' `
        'b753b14d880b9bc8d89d6acd3b665c040baec0211078435432fcae117db707af'
    $process = Start-Process -FilePath $archive -ArgumentList '-y', "-o`"$nativeRoot`"" -Wait -PassThru -WindowStyle Hidden
    if ($process.ExitCode -ne 0) { throw 'OpenCV extraction failed' }
}

$clangRoot = Join-Path $nativeRoot 'llvm-19.1.7\bin'
if (-not (Test-Path -LiteralPath "$clangRoot\libclang.dll") -or
    -not (Test-Path -LiteralPath "$clangRoot\clang.exe") -or
    -not (Test-Path -LiteralPath "$nativeRoot\llvm-19.1.7\lib\clang\19\include\stddef.h")) {
    $archive = Get-VerifiedArchive 'llvm-19.1.7.tar.xz' `
        'https://github.com/llvm/llvm-project/releases/download/llvmorg-19.1.7/clang%2Bllvm-19.1.7-x86_64-pc-windows-msvc.tar.xz' `
        'b4557b4f012161f56a2f5d9e877ab9635cafd7a08f7affe14829bd60c9d357f0'
    New-Item -ItemType Directory -Path "$nativeRoot\llvm-19.1.7" -Force | Out-Null
    tar.exe -xf $archive -C "$nativeRoot\llvm-19.1.7" --strip-components=1 `
        'clang+llvm-19.1.7-x86_64-pc-windows-msvc/bin/libclang.dll' `
        'clang+llvm-19.1.7-x86_64-pc-windows-msvc/bin/clang.exe' `
        'clang+llvm-19.1.7-x86_64-pc-windows-msvc/lib/clang/19/include'
    if ($LASTEXITCODE -ne 0) { throw 'LLVM extraction failed' }
}

$env:LIBCLANG_PATH = $clangRoot
$env:OPENCV_INCLUDE_PATHS = "$opencvRoot\include"
$env:OPENCV_LINK_PATHS = "$opencvRoot\x64\vc16\lib"
$env:OPENCV_LINK_LIBS = 'opencv_world4120'
$env:PATH = "$opencvRoot\x64\vc16\bin;$clangRoot;$env:PATH"
