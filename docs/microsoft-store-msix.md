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
  --no-bundle `
  --features microsoft-store
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

The certificate subject must match the `Publisher` value used to create the MSIX exactly. AppX
deployment validates trust at machine scope, so the disposable public certificate must be added to
both `LocalMachine\Root` and `LocalMachine\TrustedPeople`. This temporarily trusts the certificate
for every user of the test machine; use it only for this package and remove it after testing.

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

certutil.exe -f -addstore Root $cer
if ($LASTEXITCODE -ne 0) {
  throw "Adding the test certificate to LocalMachine\Root failed."
}

certutil.exe -f -addstore TrustedPeople $cer
if ($LASTEXITCODE -ne 0) {
  throw "Adding the test certificate to LocalMachine\TrustedPeople failed."
}
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

$thumbprint = $cert.Thumbprint
certutil.exe -f -delstore TrustedPeople $thumbprint
certutil.exe -f -delstore Root $thumbprint
certutil.exe -user -f -delstore My $thumbprint
```

Verify that the package and all three certificate entries are gone before leaving the elevated
session. Error `0x800B010A` or a disabled **Install** button means the package signing certificate
does not have a complete trusted chain at machine scope; do not bypass the signature check.

## Local certification result (2026-08-04)

The complete procedure above was validated on Windows 11 Pro x64 with Windows App Certification
Kit 10.0.19041.5609. The application was built with Node.js 24.15.0, pnpm 11.7.0, and Rust 1.95.0
for `x86_64-pc-windows-msvc`.

Final result:

- the release executable was confirmed as x64 and `Windows GUI`, with no console window;
- the disposable `0.1.6.0` MSIX built, signed, verified, installed, launched, restarted, and
  uninstalled successfully;
- repository selection, status, history, fetch, GitHub CLI refresh, Explorer/editor launching, and
  settings persistence passed the manual smoke test;
- WACK completed a full, non-partial run with `OVERALL_RESULT="PASS"`: 23 of 24 individual checks
  passed;
- the only individual failure was the optional **Blocked executables** check. WACK detected
  `CreateProcessW` and `ShellExecuteW` because Skibidibi Git intentionally launches the system Git
  executable, configured editors, Explorer, and optional AI CLIs. This did not change the overall
  PASS result and should be explained in Partner Center certification notes if requested;
- the generated test MSIX, public test certificate, WACK report, installed package, and all trusted
  certificate entries were removed after validation.

Issues found and corrected during the certification run:

- the Windows release validator assumed LF line endings and rejected a valid CRLF `main.rs`;
- real-Git fixtures inherited the machine-wide `core.autocrlf=true`, causing nondeterministic
  Windows failures; fixtures that depend on exact content now set local, deterministic behavior;
- canonical Windows `\\?\` paths were passed directly to `git worktree add`, which Git for Windows
  rejected; managed worktree paths are now converted to compatible drive or UNC form only at the
  Git process boundary;
- two tests assumed Unix-only file names or executable names and now accept the supported Windows
  forms without weakening their safety assertions;
- the generated manifest declared `Square310x310Logo` without the required `Wide310x150Logo`;
  the unused optional tile declaration and payload were removed;
- the Store executable now uses a dedicated `microsoft-store` build feature: GitHub Releases remain
  the update channel for standalone packages, while Store MSIX updates are delegated exclusively to
  Microsoft Store and the application only exposes bundled release notes;
- trusting the self-signed certificate only in `CurrentUser\TrustedPeople` was insufficient for
  AppX deployment and `signtool /pa`; the test procedure now uses temporary machine-scope trust and
  explicitly removes it.

The package used for this local run was test-signed and must never be uploaded to Partner Center.
For the real Store submission, run **Build Microsoft Store MSIX** from an immutable tag or commit
SHA with the exact three Product identity values, keep the workflow artifact unsigned, upload the
`.msix` on the product's **Packages** page, and let Microsoft sign it during certification.

## Partner Center resubmission checklist

Use the **MSIX or PWA app** product, not the earlier MSI/EXE product that failed with installer exit
code `1603`. Before submitting:

1. create a new release commit and immutable tag that includes the MSIX and Windows fixes (the
   `app-v0.1.6` tag predates them, so do not package that tag);
2. run **Build Microsoft Store MSIX** for that tag with the exact three values from **Product
   identity**;
3. download the unsigned `microsoft-store-msix-*` workflow artifact and upload its `.msix` file on
   Partner Center's **Packages** page;
4. verify that Partner Center reads the intended x64 architecture, version, publisher, and package
   identity before saving the submission;
5. add the following certification notes, adjusting only paths or version numbers if needed.

Suggested certification notes:

> Skibidibi Git is a full-trust desktop Git client. Git for Windows must be installed and available
> on PATH before repository operations can be tested. GitHub CLI, AI CLIs, VS Code, and Cursor are
> optional integrations and are not required to launch the app. To test: launch the app, choose a
> local Git repository, verify status and history, then use Refresh or Fetch. The application
> intentionally invokes system Git and may optionally launch Explorer, an installed editor, GitHub
> CLI, or an enabled AI CLI. This explains CreateProcessW/ShellExecuteW findings from the optional
> WACK Blocked executables check. The packaged MSIX delegates application updates to Microsoft
> Store; it does not run the standalone GitHub Releases updater.

After the final Store-feature build, repeat at least the signed disposable-package install, launch,
repository-open, restart, and uninstall smoke test. A complete WACK pass remains recommended for the
exact binary submitted to Partner Center.

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
