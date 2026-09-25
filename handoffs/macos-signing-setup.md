# Mac handoff: enable Spectrum signing and notarization

The owner wants GitHub Actions builds of `Spectrum.app` to open on their Mac
without the recurring Privacy & Security exception. Work from this repository
on the owner's Mac, using its browser, Xcode, Keychain Access, and Terminal as
needed. The owner has authorized setting up their Apple Developer account and
GitHub Actions for this purpose. Ask them to handle Apple sign-in or two-factor
prompts when required.

## Repository state

- Commit `9cc7bc3` prepared the CI signing path in `.github/workflows/ci.yml`,
  `scripts/sign-spectrum-macos.sh`, and `scripts/notarize-spectrum-macos.sh`.
- The macOS job in [CI run 36179562241](https://github.com/bentsignal/spectrum/actions/runs/36179562241)
  successfully packaged an ad hoc signed app. The Developer ID path is gated
  by a repository variable and has not run with credentials yet.
- The Linux strict timing benchmark in that run missed its budget on the
  hosted runner. The owner explicitly said not to hold up signing for this.

## One-time account and GitHub setup

1. In Apple Developer, identify the owner's 10-character Team ID. Find or
   create a **Developer ID Application** certificate for that team. Confirm
   the corresponding private key exists on this Mac. Export the identity and
   private key as a password-protected `.p12`; a certificate without its
   private key cannot sign. Keep the export outside the repository.
2. At `account.apple.com`, generate an app-specific password labelled for
   Spectrum CI notarization. Use the Apple Account associated with the team.
3. Configure these **Actions repository secrets** for `bentsignal/spectrum`:
   `SPECTRUM_MACOS_CERT_P12_BASE64` (base64 of the `.p12`),
   `SPECTRUM_MACOS_CERT_PASSWORD` (the `.p12` export password),
   `SPECTRUM_APPLE_ID` (the Apple Account email), and
   `SPECTRUM_APPLE_APP_PASSWORD` (the app-specific password). Configure the
   repository variable `SPECTRUM_APPLE_TEAM_ID` with the Team ID. GitHub's
   browser settings or `gh secret set` / `gh variable set` are both fine.
   Pass secret values directly to GitHub; never commit them, print them in
   logs, or paste them into chat.
4. Check that all four secret **names** and the Team ID variable exist. Set
   repository variable `SPECTRUM_MACOS_SIGNING_ENABLED` to `true` **last**.
   Missing credentials then cause the macOS job to fail clearly rather than
   publish another ad hoc build.

## Verify the signed build

1. Trigger `gh workflow run ci.yml --repo bentsignal/spectrum --ref main`.
   Watch the **Build macOS** job. If signing or notarization fails, report the
   run URL, failing step, and nonsecret error text. Leave this handoff in place
   so the NixOS agent can fix the CI code.
2. If the macOS job succeeds, download its `spectrum-macOS` artifact. Extract
   `Spectrum-macos-notarized.zip`, then check the extracted `Spectrum.app` with
   `codesign --verify --deep --strict --verbose=2`,
   `xcrun stapler validate`, and `spctl --assess --type execute --verbose=2`.
   Open it normally on the owner's Mac and confirm no Privacy & Security
   exception is needed. Report the run URL, Team ID, certificate fingerprint,
   and those verification results. Never report the `.p12` or passwords.

After a signed build is verified and the result has been handed back, this
handoff file is temporary: **you may delete it**, commit that deletion, and
push it to `main`. Keep the broader `tasks/macos-preview-builds.md` open for
the separate stable/development track design.

Official references: [Developer ID certificates](https://developer.apple.com/help/account/certificates/create-developer-id-certificates),
[Apple notarization](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution),
[app-specific passwords](https://support.apple.com/en-us/102654), and
[GitHub certificate secrets](https://docs.github.com/en/actions/how-tos/deploy/deploy-to-third-party-platforms/sign-xcode-applications).
