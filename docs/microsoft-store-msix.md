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

## Local testing

The Store artifact is intentionally unsigned. Windows cannot install it directly until it is signed with a locally trusted test certificate. Registering the unpacked staging directory or signing a copy with a self-signed test certificate is suitable for local tests; never upload that test-signed copy to Partner Center.

Install the current Windows SDK together with Windows App Certification Kit, then run this from an elevated PowerShell window in an active Windows user session:

```powershell
.\tools\test-microsoft-store-msix.ps1 -PackagePath .\Skibidibi.Git_0.1.6_x64_store.msix
```

The script produces `SkibidibiGit-WACK-report.xml` beside the package. WACK requires an active interactive user session, so it is intentionally not a mandatory GitHub-hosted runner step. Microsoft Store performs its own certification after submission.

## Legacy MSI failure 1603

The Store-specific Tauri configuration disables the elevated updater scheduled task. The regular MSI needs that task for per-machine self-updates, but it is an unnecessary privileged custom action during Store-controlled installation and can cause a generic MSI `1603` failure. Store builds also embed the offline WebView2 installer because downloadable bootstrap installers are prohibited.
