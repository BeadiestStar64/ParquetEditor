param(
    [Parameter(Mandatory)]
    [ValidatePattern('^[a-zA-Z0-9._-]+$')]
    [string] $Tag,
    [Parameter(Mandatory)]
    [ValidatePattern('^[a-f0-9]{40}$')]
    [string] $Commit,
    [string] $AssetDirectory = (Join-Path (Get-Location) 'dist')
)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
Set-StrictMode -Version Latest

# Refuse to publish an incomplete matrix, even if artifact download returned success.
$targets = @(
    'x86_64-pc-windows-msvc', 'aarch64-pc-windows-msvc',
    'x86_64-unknown-linux-gnu', 'aarch64-unknown-linux-gnu',
    'x86_64-apple-darwin', 'aarch64-apple-darwin'
)
$assets = @($targets | ForEach-Object {
    $extension = if ($_ -like '*windows*') { 'zip' } else { 'tar.gz' }
    $path = Join-Path $AssetDirectory "parquet-editor-$Tag-$_.$extension"
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Missing release asset: $path" }
    $path
})
$checksums = @($assets | Sort-Object | ForEach-Object {
    $hash = (Get-FileHash -LiteralPath $_ -Algorithm SHA256).Hash.ToLowerInvariant()
    "$hash  $(Split-Path $_ -Leaf)"
})
$checksumPath = Join-Path $AssetDirectory 'SHA256SUMS'
[System.IO.File]::WriteAllLines($checksumPath, $checksums, [System.Text.UTF8Encoding]::new($false))
$assets += $checksumPath

& gh release view $Tag --json tagName *> $null
$exists = $LASTEXITCODE -eq 0
if (-not $exists) {
    $notes = Join-Path $AssetDirectory 'release-notes.md'
    [System.IO.File]::WriteAllText($notes, @"
masterの更新から自動作成したリリースです。

コミット: $Commit

Windows・Ubuntu 24.04・macOS向けに、それぞれx86_64とARM64の実行ファイルを同梱しています。
WindowsはZIP、Ubuntu・macOSはtar.gzを展開し、端末から実行してください。
ファイルの検証用にSHA256SUMSを添付しています。
"@, [System.Text.UTF8Encoding]::new($false))
    & gh release create $Tag --target $Commit --title $Tag --draft --notes-file $notes
    if ($LASTEXITCODE -ne 0) { throw 'Could not create release draft' }
}
& gh release upload $Tag @assets --clobber
if ($LASTEXITCODE -ne 0) { throw 'Asset upload failed; release was not published' }

# An older build finishing later must not replace a newer master release as Latest.
$head = & gh api 'repos/{owner}/{repo}/git/ref/heads/master' --jq '.object.sha'
if ($LASTEXITCODE -ne 0) { throw 'Could not resolve master before publishing' }
$latest = if ($head.Trim() -eq $Commit) { '--latest' } else { '--latest=false' }
& gh release edit $Tag --draft=false $latest
if ($LASTEXITCODE -ne 0) { throw 'Could not publish release' }
