# Microsoft Store MSIX release

Skibidibi Git uses an MSIX package for Microsoft Store distribution. Microsoft signs the package after certification, so this path does not require a commercial Authenticode certificate.

## One-time Partner Center setup

Create a product using **New product → MSIX or PWA app**. An MSI/EXE product cannot be converted into an MSIX product.

Open **Product management → Product identity** and copy these values exactly:

- `Package/Identity/Name`
- `Package/Identity/Publisher` (the value beginning with `CN=`)
- `Package/Properties/PublisherDisplayName`

The public Product ID is not a replacement for any of these values.

## Build

Run the `Build Microsoft Store MSIX` GitHub workflow and provide an immutable release tag or commit SHA plus all three identity values. `main` is acceptable only for a disposable packaging test. The workflow:

1. checks out the selected source revision;
2. validates and builds the Angular/Tauri application for Windows x64;
3. creates an unsigned full-trust MSIX package with the Partner Center identity;
4. uploads the MSIX as a workflow artifact.

Upload that `.msix` file on the new product's **Packages** page. Do not use a package URL; URLs belong to the MSI/EXE submission path.

## Complete local Windows test (copy and paste)

The following procedure builds a disposable MSIX, signs it with a locally trusted self-signed certificate, installs it, and runs Windows App Certification Kit. Never upload the test-signed package to Partner Center. The Store artifact must remain unsigned so Microsoft can sign it after certification.

### Prerequisites

Install these components first:

- Git;
- Node.js 22;
- pnpm 11.7 (`corepack enable`, then `corepack prepare pnpm@11.7.0 --activate`);
- Rust through rustup with the stable MSVC toolchain;
- Visual Studio Build Tools with **Desktop development with C++**;
- current Windows SDK with **Windows App Certification Kit** and signing tools;
- Microsoft Edge WebView2 Runtime (normally already installed on supported Windows versions).

Open **PowerShell as Administrator**, enter the repository, and allow repository scripts for this process only:

```powershell
cd C:\projects\skibidibi-git
Set-ExecutionPolicy -Scope Process Bypass

git fetch origin
git switch codex/microsoft-store-msix
git pull
pnpm install

node --version
pnpm --version
rustc --version
cargo --version
```

### 1. Build the Tauri executable

```powershell
pnpm tauri build `
  --target x86_64-pc-windows-msvc `
  --no-bundle
```

If the build cannot find `link.exe`, reopen Visual Studio Installer and add **Desktop development with C++**.

### 2. Create a disposable MSIX

The identity and publisher below are intentionally local test values. They are not Partner Center production identity values.

```powershell
$msix = Join-Path $PWD "Skibidibi.Git_0.1.6_x64_test.msix"

.\tools\build-microsoft-store-msix.ps1 `
  -IdentityName "Rbrzoska.SkibidibiGit.Test" `
  -Publisher "CN=Skibidibi Git Test" `
  -PublisherDisplayName "Rafal Brzoska" `
  -OutputPath $msix

Get-Item $msix
```

### 3. Create and trust a free development certificate

The certificate subject must match the `Publisher` value used to create the MSIX exactly.

```powershell
$cert = New-SelfSignedCertificate `
  -Type Custom `
  -KeyUsage DigitalSignature `
  -Subject "CN=Skibidibi Git Test" `
  -FriendlyName "Skibidibi Git development" `
  -CertStoreLocation "Cert:\CurrentUser\My" `
  -TextExtension @(
    "2.5.29.37={text}1.3.6.1.5.5.7.3.3",
    "2.5.29.19={text}"
  )

$cer = Join-Path $PWD "SkibidibiGit-Test.cer"
Export-Certificate -Cert $cert -FilePath $cer | Out-Null
Import-Certificate `
  -FilePath $cer `
  -CertStoreLocation "Cert:\CurrentUser\TrustedPeople" | Out-Null
```

### 4. Sign and verify the test package

```powershell
$signTool = Get-ChildItem `
  "${env:ProgramFiles(x86)}\Windows Kits\10\bin" `
  -Filter signtool.exe `
  -Recurse |
  Where-Object { $_.FullName -match '\\x64\\signtool\.exe$' } |
  Sort-Object FullName -Descending |
  Select-Object -First 1 -ExpandProperty FullName

if (-not $signTool) {
  throw "signtool.exe was not found. Install the current Windows SDK signing tools."
}

& $signTool sign `
  /fd SHA256 `
  /sha1 $cert.Thumbprint `
  /s My `
  $msix

if ($LASTEXITCODE -ne 0) {
  throw "Signing failed with exit code $LASTEXITCODE."
}

& $signTool verify /pa /v $msix
if ($LASTEXITCODE -ne 0) {
  throw "Signature verification failed with exit code $LASTEXITCODE."
}
```

### 5. Install and launch

```powershell
Add-AppxPackage $msix
Get-AppxPackage -Name "Rbrzoska.SkibidibiGit.Test"
```

Launch **Skibidibi Git** from the Start menu. Verify at least:

- the app starts without a console window;
- a repository folder can be selected;
- status and commit history load;
- Git fetch works;
- GitHub CLI integration can be refreshed;
- Finder/Explorer and configured editors can be opened;
- settings persist after an app restart.

### 6. Run Windows App Certification Kit

Run this in the same elevated, interactive PowerShell session:

```powershell
.\tools\test-microsoft-store-msix.ps1 -PackagePath $msix
```

The script writes `SkibidibiGit-WACK-report.xml` beside the package. Keep the complete report when a check fails. WACK requires an active user session, which is why it is not a mandatory GitHub-hosted runner step.

### 7. Remove the test installation and certificate

Run this only after testing is complete:

```powershell
Get-AppxPackage -Name "Rbrzoska.SkibidibiGit.Test" |
  Remove-AppxPackage

Remove-Item "Cert:\CurrentUser\TrustedPeople\$($cert.Thumbprint)" -ErrorAction SilentlyContinue
Remove-Item "Cert:\CurrentUser\My\$($cert.Thumbprint)" -ErrorAction SilentlyContinue
```

## Legacy MSI failure 1603

The Store-specific Tauri configuration disables the elevated updater scheduled task. The regular MSI needs that task for per-machine self-updates, but it is an unnecessary privileged custom action during Store-controlled installation and can cause a generic MSI `1603` failure. Store builds also embed the offline WebView2 installer because downloadable bootstrap installers are prohibited.

To reproduce a Store-style silent MSI installation and capture the actual failing action, run this from an elevated PowerShell session after replacing the path:

```powershell
$msi = "$env:USERPROFILE\Downloads\Skibidibi.Git_0.1.6_x64_en-US.msi"
$log = "$env:USERPROFILE\Desktop\skibidibi-install.log"

$process = Start-Process msiexec.exe `
  -ArgumentList @(
    "/i",
    "`"$msi`"",
    "/qn",
    "/norestart",
    "/L*v",
    "`"$log`""
  ) `
  -Wait `
  -PassThru

$process.ExitCode
Select-String -Path $log -Pattern "Return value 3" -Context 40,15
```

Preserve the full `skibidibi-install.log`; error `1603` alone is only a generic MSI failure code.
