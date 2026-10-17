param(
    [Parameter(Mandatory)]
    [ValidateSet('x86_64-pc-windows-msvc', 'aarch64-pc-windows-msvc', 'x86_64-unknown-linux-gnu', 'aarch64-unknown-linux-gnu', 'x86_64-apple-darwin', 'aarch64-apple-darwin')]
    [string] $Target,
    [Parameter(Mandatory)]
    [ValidatePattern('^[a-zA-Z0-9._-]+$')]
    [string] $Tag,
    [string] $RepositoryRoot = (Split-Path (Split-Path $PSScriptRoot -Parent) -Parent),
    [string] $OutputDirectory = (Join-Path $RepositoryRoot 'dist')
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$isWindowsTarget = $Target -like '*windows*'
$binary = if ($isWindowsTarget) { 'parquet-editor.exe' } else { 'parquet-editor' }
$source = Join-Path $RepositoryRoot "target/$Target/release/$binary"
if (-not (Test-Path -LiteralPath $source -PathType Leaf)) {
    throw "Release binary not found: $source"
}
$name = "parquet-editor-$Tag-$Target"
$staging = Join-Path $OutputDirectory "staging/$name"
New-Item -ItemType Directory -Force -Path $staging | Out-Null
Copy-Item -LiteralPath $source -Destination (Join-Path $staging $binary)
Copy-Item -LiteralPath (Join-Path $RepositoryRoot 'README.md') -Destination $staging
New-Item -ItemType Directory -Force -Path (Join-Path $staging 'translate') | Out-Null
Copy-Item -Path (Join-Path $RepositoryRoot 'translate/*.json') -Destination (Join-Path $staging 'translate')

if ($isWindowsTarget) {
    $archive = Join-Path $OutputDirectory "$name.zip"
    [System.IO.Compression.ZipFile]::CreateFromDirectory($staging, $archive)
} else {
    $archive = Join-Path $OutputDirectory "$name.tar.gz"
    # tar stores executable permissions, unlike the intermediate Actions artifact ZIP.
    if (-not $IsWindows) {
        & chmod +x (Join-Path $staging $binary)
        if ($LASTEXITCODE -ne 0) { throw 'chmod failed' }
    }
    & tar -czf $archive -C $staging .
    if ($LASTEXITCODE -ne 0) { throw 'tar failed' }
}
Write-Output "Packaged $archive"
