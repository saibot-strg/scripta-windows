# Distribution: installer, SmartScreen, Smart App Control, code signing

## What the pipeline builds

`.github/workflows/release.yml` builds an NSIS installer on `windows-latest` for every `v*` tag and attaches it
(with a SHA-256 checksum) to a GitHub release. The installer:

- installs per user (`%LOCALAPPDATA%\Programs\Scripta`), no administrator rights,
- adds a Start menu entry and an uninstaller,
- downloads the WebView2 runtime only if it is missing (Windows 11 ships it).

Nothing else needs to be installed or started by hand; the app downloads its speech models on first start.

## Unsigned builds today

| Protection | What users see | Way around |
|---|---|---|
| SmartScreen (all PCs) | "Windows protected your PC" for a new, unsigned download | **More info → Run anyway** |
| Smart App Control (Windows 11 PCs that were set up fresh, unmanaged) | the installer and the app are blocked, no way to run them | only code signing (or turning Smart App Control off, which can't be undone) |
| Company policies (WDAC/AppLocker, Intune) | depends on IT | IT allows the app by publisher or hash |

Smart App Control was found on the development PC on 2026-10-23: from that moment, every freshly built
unsigned executable was blocked (Code Integrity events 3033/3077).

## Recommendation for the office rollout

1. **Code signing.** Any certificate that chains to a root in Microsoft's trusted root program satisfies
   Smart App Control, and SmartScreen warnings disappear once the publisher has reputation.
   - *Azure Trusted Signing* (Microsoft): about $10 per month, certificates renewed automatically,
     integrates with GitHub Actions (`azure/trusted-signing-action`). Requires an organization (or an
     individual developer, depending on the country) to be verified by Microsoft.
   - An OV code signing certificate from a CA (on a hardware token or cloud HSM): about €200–400 per year.
   - Tauri signs with `bundle.windows.signCommand` in `tauri.conf.json`; the pipeline needs the
     credentials as repository secrets.
2. **Through IT**: if colleagues' PCs are managed (Intune, software center), IT can distribute the installer
   and allow it by publisher or hash. Then only the release on GitHub is needed.

Nothing in the app changes for signing; it is a pipeline step.
