param(
  [Parameter(Mandatory = $true)][string]$IdentityName,
  [Parameter(Mandatory = $true)][string]$Publisher,
  [Parameter(Mandatory = $true)][string]$PublisherDisplayName,
  [Parameter(Mandatory = $true)][string]$OutputPath
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$temporaryRoot = if ($env:RUNNER_TEMP) { $env:RUNNER_TEMP } else { $env:TEMP }
if (-not $temporaryRoot) {
  throw "Neither RUNNER_TEMP nor TEMP is available."
}
$stage = Join-Path $temporaryRoot "skibidibi-git-msix"
$executableName = "SkibidibiGit.exe"

if (Test-Path $stage) {
  Remove-Item $stage -Recurse -Force
}
New-Item -ItemType Directory -Path (Join-Path $stage "Assets") -Force | Out-Null

$candidateRoots = @(
  (Join-Path $repoRoot "target\x86_64-pc-windows-msvc\release"),
  (Join-Path $repoRoot "apps\desktop\src-tauri\target\x86_64-pc-windows-msvc\release")
)
$application = $candidateRoots |
  ForEach-Object { Join-Path $_ "skibidibi-git-desktop.exe" } |
  Where-Object { Test-Path $_ } |
  Select-Object -First 1
if (-not $application) {
  throw "The compiled Tauri executable was not found. Build it before creating the MSIX package."
}

Copy-Item $application (Join-Path $stage $executableName)
$applicationDirectory = Split-Path -Parent $application
Get-ChildItem $applicationDirectory -File -Filter "*.dll" |
  ForEach-Object { Copy-Item $_.FullName (Join-Path $stage $_.Name) }
$icons = @("StoreLogo.png", "Square44x44Logo.png", "Square150x150Logo.png", "Square310x310Logo.png")
foreach ($icon in $icons) {
  Copy-Item (Join-Path $repoRoot "apps\desktop\src-tauri\icons\$icon") (Join-Path $stage "Assets\$icon")
}

node (Join-Path $PSScriptRoot "create-microsoft-store-msix-manifest.mjs") `
  --output (Join-Path $stage "AppxManifest.xml") `
  --identity-name $IdentityName `
  --publisher $Publisher `
  --publisher-display-name $PublisherDisplayName `
  --architecture x64 `
  --executable $executableName
if ($LASTEXITCODE -ne 0) {
  throw "Generating AppxManifest.xml failed."
}

$makeAppx = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin" -Filter makeappx.exe -Recurse |
  Where-Object { $_.FullName -match '\\x64\\makeappx\.exe$' } |
  Sort-Object FullName -Descending |
  Select-Object -First 1
if (-not $makeAppx) {
  throw "MakeAppx.exe was not found in the Windows SDK."
}

$outputDirectory = Split-Path -Parent $OutputPath
New-Item -ItemType Directory -Path $outputDirectory -Force | Out-Null
& $makeAppx.FullName pack /o /v /h SHA256 /d $stage /p $OutputPath
if ($LASTEXITCODE -ne 0) {
  throw "MakeAppx failed with exit code $LASTEXITCODE."
}

Write-Host "Created unsigned Microsoft Store package: $OutputPath"
