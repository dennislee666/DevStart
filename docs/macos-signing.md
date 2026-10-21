# Signed macOS releases

Release downloads are built on GitHub Actions from the public `open-source` branch. The Universal executable contains Intel and Apple Silicon slices and retains the macOS 12.0 minimum. Public packages never include a private workspace catalog or launch history.

## Certificate and notarization requirements

A `Developer ID Application` certificate is required for downloads outside the Mac App Store. An `Apple Distribution` or `Mac App Distribution` certificate is not a substitute. DMG/ZIP distribution does not require a Developer ID Installer certificate (that certificate is for PKG installers).

The team's Account Holder creates Developer ID Application certificates in the Apple Developer portal. Use a locally generated CSR and retain the matching private key securely. Do not commit CSR signing private keys, P12/P8 files, passwords, or account tokens to either source repository.

Store the following as encrypted GitHub Actions repository/environment secrets:

| Secret | Purpose |
|---|---|
| `MACOS_CERTIFICATE_P12_BASE64` | Base64 of the password-protected certificate and matching private key |
| `MACOS_CERTIFICATE_PASSWORD` | P12 import password |
| `MACOS_SIGNING_IDENTITY` | Developer ID Application subject or SHA-1 fingerprint |
| `APPLE_TEAM_ID` | Developer team identifier |

Configure one notarization credential method:

- API key: `APPLE_API_KEY_P8_BASE64`, `APPLE_API_KEY_ID`, and `APPLE_API_ISSUER_ID` for a team key (omit issuer for an individual key supported by the installed notarytool).
- Apple ID: `APPLE_ID` and `APPLE_APP_SPECIFIC_PASSWORD`. Use an app-specific password, never the main Apple account password.

Partial API key configuration is an error and does not silently switch authentication methods.

## Pipeline

1. Validate required credential configuration before the long build.
2. Run Rust and release-tool checks and build both CPU architectures.
3. Verify edition, version, architectures, signing structure and local APIs.
4. Import the certificate into an isolated temporary keychain, enable hardened runtime and timestamp-sign the application. Verify Developer ID authority and expected team.
5. Notarize the app ZIP, require `Accepted`, staple the application ticket, and assess the app with Gatekeeper.
6. Create the DMG with an Applications shortcut, sign and notarize it, staple and validate its ticket, and assess the disk image.
7. Create the final ZIP from the stapled app. Verify DMG/ZIP integrity and generate checksums after all signing/stapling operations.
8. Publish the verified stable version through `publish-release.py`; only then remove intermediate releases of the same version.

Missing credentials, wrong identity/team, rejected notarization, stapling errors and Gatekeeper failures all stop publication. There is no unsigned fallback. Notarization diagnostics are uploaded separately on failure for seven days; package assets contain only the DMG, ZIP and checksum manifest. Temporary keychain/search-list changes and credential files are cleaned up even on failure.

[Apple Developer ID](https://developer.apple.com/help/account/certificates/create-developer-id-certificates/) · [Apple notarization](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution) · [GitHub signing guidance](https://docs.github.com/en/actions/how-tos/deploy/deploy-to-third-party-platforms/sign-xcode-applications)
