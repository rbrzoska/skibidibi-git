param(
  [Parameter(Mandatory = $true)][string]$PackagePath,
  [string]$ReportPath = ""
)

$ErrorActionPreference = "Stop"
$resolvedPackage = (Resolve-Path $PackagePath).Path
if (-not $ReportPath) {
  $ReportPath = Join-Path (Split-Path -Parent $resolvedPackage) "SkibidibiGit-WACK-report.xml"
}

$appCert = Join-Path ${env:ProgramFiles(x86)} "Windows Kits\10\App Certification Kit\appcert.exe"
if (-not (Test-Path $appCert)) {
  throw "Windows App Certification Kit was not found. Install the current Windows SDK with WACK, then retry."
}

Write-Host "Resetting Windows App Certification Kit state..."
& $appCert reset
if ($LASTEXITCODE -ne 0) {
  throw "appcert reset failed with exit code $LASTEXITCODE."
}

Write-Host "Testing $resolvedPackage..."
& $appCert test -appxpackagepath $resolvedPackage -reportoutputpath $ReportPath
if ($LASTEXITCODE -ne 0) {
  throw "Windows App Certification Kit failed with exit code $LASTEXITCODE. Review $ReportPath."
}

Write-Host "Windows App Certification Kit passed. Report: $ReportPath"
