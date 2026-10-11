# Platform code signing: macOS, Windows, Linux and peer practice

Survey of October 11, 2026. It gathers the facts a signing decision needs for the Goodfetch release: the `goodfetch` CLI, the Windows filter broker service, the Linux `goodfetch-sandbox` helper and the Goodfetch-built Chromium. Every claim links its source. **UNVERIFIED** marks what no primary source confirmed. Statements from Apple DTS engineers in the Apple Developer Forums count as vendor statements. Statements from other forum posts are marked as reports.

Workflow files were read on `main`/`trunk` on the survey date. The links point to the branch, not to commit permalinks, so they can drift.

## 1. macOS

### When Gatekeeper and notarization apply

- Gatekeeper normally checks only quarantined code. An Apple DTS engineer says Gatekeeper "usually only kicks in" when the app is quarantined. The same engineer says most Unix-style tools, curl and scp included, don't quarantine their downloads. [Apple forums 666452](https://developer.apple.com/forums/thread/666452). Gatekeeper does run in some cases without quarantine, and the same thread says Homebrew formulae aren't quarantined. [Apple forums 729336](https://developer.apple.com/forums/thread/729336).
- Who sets the `com.apple.quarantine` attribute:
  - **Browsers:** Chromium sets it through `NSURLQuarantinePropertiesKey`. [quarantine_mac.mm](https://github.com/chromium/chromium/blob/main/components/services/quarantine/quarantine_mac.mm). Safari doing the same is well known but has no primary source here (**UNVERIFIED**).
  - **Apps that opt in:** an app with `LSFileQuarantineEnabled` quarantines the files it creates. [LSFileQuarantineEnabled](https://developer.apple.com/documentation/bundleresources/information-property-list/lsfilequarantineenabled). A file created by such an app makes Terminal stop with a Gatekeeper alert. [Apple forums 725487](https://developer.apple.com/forums/thread/725487).
  - **curl:** doesn't set it, per the Apple DTS statement above.
  - **npm and pip:** no primary source found. Both are Unix-style tools, so by the DTS rule they shouldn't set it (**UNVERIFIED**).
  - **A Rust HTTP client in a process that doesn't opt in:** this follows from the two Apple sources above; Apple doesn't say it directly. Chrome for Testing's README says downloads through `@puppeteer/browsers`, curl or wget avoid the "damaged app" problem. [CfT README](https://github.com/GoogleChromeLabs/chrome-for-testing/blob/main/README.md#macos-says-the-app-is-damaged-what-now).
  - **Inheritance:** one third-party report says processes launched from a quarantine-enabled GUI app inherit quarantine on the files they write. [nub PR 601](https://github.com/nubjs/nub/pull/601) (**UNVERIFIED** by Apple).
- **Homebrew casks** are quarantined on purpose: Homebrew "applies macOS quarantine attributes to Cask downloads so Gatekeeper performs these checks". [Homebrew security docs](https://docs.brew.sh/Homebrew-Security-and-Supply-Chain), [quarantine.rb](https://github.com/Homebrew/brew/blob/main/Library/Homebrew/extend/os/mac/cask/quarantine.rb).
- **Homebrew formulae** are not quarantined. [Apple forums 729336](https://developer.apple.com/forums/thread/729336).
- **A quarantined, un-notarized bare CLI started from Terminal** is blocked with "the identity of the developer cannot be confirmed". This is a developer report, not an Apple statement. [Apple forums 757647](https://developer.apple.com/forums/thread/757647). An Apple DTS engineer says quarantine "tells Gatekeeper to check the program before allowing it to start". [Apple forums 773755](https://developer.apple.com/forums/thread/773755).
- **A notarized bare tool** runs from Terminal but is refused when double-clicked in Finder. Apple lists this as a known bug. [Apple forums 706379](https://developer.apple.com/forums/thread/706379).
- **macOS Sequoia (15)** removed the Control-click override. Users now have to go to System Settings > Privacy & Security. [Apple developer news, 2024-08-06](https://developer.apple.com/news/?id=saqachfa), [Apple support 102445](https://support.apple.com/en-us/102445).
- **macOS Tahoe (26):** no Apple announcement of a Gatekeeper policy change was found; only Gatekeeper-bypass security fixes appear, for example in 26.2. [Apple support 125886](https://support.apple.com/en-us/125886). Reports of stricter enforcement come only from secondary sites (**UNVERIFIED**).

### Apple Silicon ad-hoc signing

- "A Mac with Apple silicon doesn't permit native arm64 code to execute unless a valid signature is attached." An ad-hoc signature is enough. [Apple Platform Security: Rosetta 2](https://support.apple.com/guide/security/rosetta-2-on-a-mac-with-apple-silicon-secebb113be1/web).
- The linker ad-hoc signs automatically. Tools that change a binary after linking, such as `strip` or `install_name_tool`, may require calling `codesign` again. Ad-hoc signed code cannot pass Gatekeeper. [macOS Big Sur 11.0.1 universal apps release notes](https://developer.apple.com/documentation/macos-release-notes/macos-big-sur-11_0_1-universal-apps-release-notes).
- So any post-link patching of the Chromium binaries has to be followed by re-signing.

### Hardened runtime and JIT entitlements

- To be notarized, code needs the hardened runtime, a Developer ID certificate, a secure timestamp, and no `com.apple.security.get-task-allow` entitlement. [Hardened Runtime](https://developer.apple.com/documentation/security/hardened-runtime), [Notarizing macOS software](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution).
- Relevant entitlements:
  - `com.apple.security.cs.allow-jit` allows `MAP_JIT` memory. [allow-jit](https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.security.cs.allow-jit)
  - `allow-unsigned-executable-memory` is meant for "rare cases". [allow-unsigned-executable-memory](https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.security.cs.allow-unsigned-executable-memory)
  - `disable-library-validation` triggers extra Gatekeeper checks. [disable-library-validation](https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.security.cs.disable-library-validation)
- What Chrome itself uses:
  - Only the Renderer and GPU helpers carry `com.apple.security.cs.allow-jit`. [helper-renderer-entitlements.plist](https://chromium.googlesource.com/chromium/src/+/main/chrome/app/helper-renderer-entitlements.plist).
  - Those helpers are signed without the library-validation flag, because Chromium's signing script says "library validation is incompatible with the JIT entitlement". Every other part gets the full hardened-runtime flag set. [parts.py](https://github.com/chromium/chromium/blob/main/chrome/installer/mac/signing/parts.py).
  - The main app's entitlements cover only device and privacy access. [app-entitlements.plist](https://chromium.googlesource.com/chromium/src/+/main/chrome/app/app-entitlements.plist).
  - ungoogled-chromium-macos applies the same scheme, and its builds are notarized and stapled. [sign_and_package_app.sh](https://github.com/ungoogled-software/ungoogled-chromium-macos/blob/master/sign_and_package_app.sh).

### Notarizing a bare Mach-O CLI

- The notary service accepts UDIF disk images, signed flat `.pkg` files and ZIP archives. A ticket can't be stapled to a ZIP, and although tickets are created for standalone binaries, "it's not currently possible to staple tickets to them". Gatekeeper looks the ticket up online. [Customizing the notarization workflow](https://developer.apple.com/documentation/security/customizing-the-notarization-workflow).
- A bare CLI can be notarized inside a ZIP, but its first run when offline and quarantined may be refused. [Apple forums 701581](https://developer.apple.com/forums/thread/701581).
- `rcodesign` follows the same rules: it refuses to staple a Mach-O binary. [notarization.rs](https://github.com/indygreg/apple-platform-rs/blob/main/apple-codesign/src/notarization.rs), [stapling.rs](https://github.com/indygreg/apple-platform-rs/blob/main/apple-codesign/src/stapling.rs).

### Homebrew policy 2025–2026

- **Homebrew 5.0.0 (2025-11-12):**
  - "Casks without codesigning are deprecated."
  - `--no-quarantine` and `--quarantine` are deprecated, "as Homebrew does not wish to easily provide circumvention to macOS security features".
  - "We will disable all Homebrew/homebrew-cask casks that fail Gatekeeper checks in September 2026."
  - [Homebrew 5.0.0](https://brew.sh/2025/11/12/homebrew-5.0.0/), cutoff 2026-09-01 tracked in [brew#20755](https://github.com/Homebrew/brew/issues/20755).
- The flag is still present (deprecated) in `cmd/install.rb` at tag 5.0.0 and gone at tag 6.0.0. This was checked in the brew source at those tags.
- **Homebrew 6.0.0 (2026-06-11):** third-party taps must be "explicitly trusted before their code is evaluated"; official taps stay trusted by default. [Homebrew 6.0.0](https://brew.sh/2026/06/11/homebrew-6.0.0/). The current release is 7.0.9 (2026-10-08). [brew releases](https://github.com/Homebrew/brew/releases).
- **Scope:** the Gatekeeper policy covers the official homebrew/cask repository. A maintainer says it "doesn't stop anyone from maintaining their own tap full of unsigned software". [Homebrew discussion 6482](https://github.com/orgs/Homebrew/discussions/6482).
- Formulae are not quarantined (see above), so the policy doesn't affect them.
- Acceptable Casks requires passing Gatekeeper, and says open-source CLI-only software "normally belongs in homebrew/core as a formula". [Acceptable Casks](https://docs.brew.sh/Acceptable-Casks).
- Whether the September 2026 disables were actually carried out was not checked (**UNVERIFIED**). The `ungoogled-chromium` cask has no `disable!` line, which fits its notarized DMG. [cask file](https://github.com/Homebrew/homebrew-cask/blob/main/Casks/u/ungoogled-chromium.rb).

### Notarizing from GitHub Actions

- **`xcrun notarytool`** (macOS runner) accepts an App Store Connect API key through `--key`, `--key-id` and `--issuer`. This was checked with the local `notarytool submit --help`. Apple's Notary REST API takes the same key as a JWT, "to avoid a macOS dependency". [Notary API](https://developer.apple.com/documentation/notaryapi/submitting-software-for-notarization-over-the-web).
- **`rcodesign`** (the `apple-codesign` crate):
  - Signs Mach-O files, bundles, `.pkg` and `.dmg` from non-Apple systems such as Linux and Windows as well as macOS, and notarizes and staples bundles, `.pkg` and `.dmg`, using an App Store Connect API key. [apple-codesign docs](https://gregoryszorc.com/docs/apple-codesign/main/), [getting started](https://gregoryszorc.com/docs/apple-codesign/main/apple_codesign_getting_started.html).
  - `--code-signature-flags runtime` turns on the hardened runtime. [signing](https://gregoryszorc.com/docs/apple-codesign/main/apple_codesign_rcodesign_signing.html).
  - A GitHub Action exists. [GitHub Actions](https://gregoryszorc.com/docs/apple-codesign/main/apple_codesign_github_actions.html).
  - The last tagged release is 0.29.0 (2024-11-29); the repository still gets commits. [releases](https://github.com/indygreg/apple-platform-rs/releases).
- Astral (uv, ruff) and OpenAI (Codex) run `rcodesign` on Linux runners, with the key in Azure Key Vault reached through PKCS#11 (see section 3).

### Apple Developer Program enrollment

- **Individual:**
  - Enrolls under their legal name, with no D-U-N-S number.
  - "Your name will be displayed as the seller name."
  - [Enroll](https://developer.apple.com/programs/enroll/), [D-U-N-S](https://developer.apple.com/help/account/membership/D-U-N-S/).
- **Organization:**
  - Must be "a legal entity that can enter into contracts"; DBAs and trade names are refused.
  - Needs a D-U-N-S number (free).
  - Needs a public website on its own domain and an email on that domain.
  - The person enrolling must have authority to bind the organization.
  - [Enroll](https://developer.apple.com/programs/enroll/).
  - A GitHub organization isn't a legal entity, so it can't enroll.
  - Whether a French *association loi 1901* is accepted was not confirmed (**UNVERIFIED**).
- **Name on the certificate:**
  - The certificate reads "Developer ID Application: <team name> (<TEAMID>)". [Apple forums 811970](https://developer.apple.com/forums/thread/811970). For an individual, that is their personal name; ungoogled-chromium shows "Qian Qian (B9A88FL5XJ)". [ungoogled-chromium-macos README](https://github.com/ungoogled-software/ungoogled-chromium-macos).
  - Only the Account Holder can create Developer ID certificates. [Create Developer ID certificates](https://developer.apple.com/help/account/certificates/create-developer-id-certificates/).
  - The original Developer ID intermediate expires 2027-02-01, and new G2 certificates are valid for one year. [Replace Developer ID certificates](https://developer.apple.com/help/account/certificates/replace-developer-id-certificates).
- **Cost:** 99 USD a year, billed in local currency. [Enroll](https://developer.apple.com/programs/enroll/). The current EUR price was not found on an Apple page (**UNVERIFIED**).
- **Fee waiver:** available to nonprofits "officially recognized by a relevant authority", for example an EU charity register; individuals are excluded. [Fee waivers](https://developer.apple.com/help/account/membership/fee-waivers/). Whether France is eligible is **UNVERIFIED**; only 2020 French press reports say so.

### Policy risk for a fingerprint-spoofing browser

- Notarization "is not App Review": it is an automated scan for malicious content. [Notarizing macOS software](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution).
- The Apple Developer Program License Agreement [(ADPLA)](https://developer.apple.com/support/terms/apple-developer-program-license-agreement/) gives Apple broad discretion:
  - **§5.3:** "in limited cases, a manual, technical investigation"; tickets can be revoked for malware or "malicious, suspicious or harmful code"; you must not "hide, attempt to bypass, or misrepresent" your application to Apple's security checks.
  - **§5.4:** certificates can be revoked "at any time, in its sole discretion", including for products that "adversely affect the security of… systems, or networks".
  - **§3.2:** you must not interfere with macOS "security… verification or authentication mechanisms", and must not facilitate "a crime, or other tortious, unlawful or illegal act".
  - **§3.3** (purpose restrictions such as not "violating the legal rights of others") covers App Store, Custom App, TestFlight and Ad Hoc distribution, not Developer ID distribution.
- No case was found of Apple revoking a Developer ID for an anti-detect browser or a scraper. The revocations found are malware cases. [AppleInsider, 2025-12-23](https://appleinsider.com/articles/25/12/23/malware-bypassed-macos-gatekeeper-by-abusing-apples-notarization-proccess).
- ungoogled-chromium, a privacy-patched Chromium, is notarized today. Whether fingerprint-chromium's macOS builds are notarized is **UNVERIFIED**. [fingerprint-chromium](https://github.com/adryfish/fingerprint-chromium).

## 2. Windows

### SmartScreen and Mark-of-the-Web

- SmartScreen warns on a downloaded file that is neither known-bad nor well-known. [Microsoft Defender SmartScreen](https://learn.microsoft.com/en-us/windows/security/operating-system-security/virus-and-threat-protection/microsoft-defender-smartscreen/).
- Unsigned files show "Windows protected your PC" and need "Run anyway". Each new unsigned version starts again from zero reputation. [SmartScreen reputation](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/smartscreen-reputation).
- Who writes Mark-of-the-Web (MotW, the `Zone.Identifier` data stream):
  - Browsers do: Chrome through IAttachmentExecute, Firefox by writing it directly.
  - curl, bitsadmin, and tools built on WinINET, WinHTTP or System.Net, PowerShell included, do not. This is by design, per a former Microsoft engineer. [textslashplain, 2016](https://textslashplain.com/2016/04/04/downloads-and-the-mark-of-the-web).
  - PowerShell's source writes `Zone.Identifier` only in `Unblock-File`, zone-reading code and tests; Invoke-WebRequest and Invoke-RestMethod don't. [PowerShell repo](https://github.com/PowerShell/PowerShell) (code search).
  - curl writes it on Windows only with the opt-in `--xattr` flag, from curl 8.22.0. [curl --xattr](https://github.com/curl/curl/blob/master/docs/cmdline-opts/xattr.md).
  - The default branches of npm/cli, pip and reqwest have no `Zone.Identifier` code. This is weaker evidence: GitHub code search only covers default branches.
- SmartScreen's check runs when a marked file is launched through Explorer/ShellExecute. A console exe started by `CreateProcess` from a terminal, or a service, gets no SmartScreen prompt. [textslashplain](https://textslashplain.com/2016/04/04/downloads-and-the-mark-of-the-web). Microsoft Learn doesn't state this explicitly (**UNVERIFIED** in Microsoft docs).

### Smart App Control (SAC)

- SAC combines Microsoft's cloud app intelligence with code integrity. When the cloud service can't predict a file, SAC allows it if it is signed by a Trusted Root Program CA. Unknown unsigned code is blocked in enforcement mode. [SAC overview (2026-09-30)](https://learn.microsoft.com/en-us/windows/apps/develop/smart-app-control/overview).
- SAC ignores MotW: "Smart App Control signature checks apply to all executable files, not just those downloaded from the Internet." [SmartScreen reputation](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/smartscreen-reputation).
- So an unsigned `goodfetch.exe`, an unsigned broker service, or unsigned Chromium exes and DLLs would be blocked on a machine where SAC enforces, however they were installed.
- **Turning SAC back on:** Windows 11 builds 26100.7701 and 26200.7701 (KB5074105, Release Preview, January 2026) let users turn SAC on or off "without any clean install requirement", rolled out gradually. [Windows Insider blog, 2026-01-27](https://blogs.windows.com/windows-insider/2026/01/27/releasing-windows-11-builds-26100-7701-and-26200-7701-to-the-release-preview-channel/).
- The current overview page no longer mentions the clean-install requirement. Whether it still applies to the first evaluation is **UNVERIFIED**.
- The share of machines with SAC enforcing is not published.

### EV vs OV certificates

- Since 2024, EV certificates no longer bypass SmartScreen: Microsoft removed the EV-specific identifiers from its Trusted Root Program. Reputation builds from the file hash plus the publisher certificate. [Distribution feature status](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/distribution-feature-status).
- Microsoft calls OV "functionally equivalent" to Artifact Signing for SmartScreen, and says paying for EV only to avoid warnings is "no longer justified". [Code signing options](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/code-signing-options).

### Microsoft Artifact Signing (formerly Trusted Signing)

- **Name:** Trusted Signing was renamed Artifact Signing and became generally available in January 2026, with no functional change. [Microsoft Security blog](https://techcommunity.microsoft.com/blog/microsoft-security-blog/-/4482789), [overview](https://learn.microsoft.com/en-us/azure/artifact-signing/overview).
- **Price:** Basic is $9.99/month for 5,000 signatures; Premium is $99.99/month for 100,000 signatures; extra signatures cost $0.005 each. It needs a paid Azure subscription. [pricing](https://azure.microsoft.com/en-us/pricing/details/artifact-signing/), [FAQ](https://learn.microsoft.com/en-us/azure/artifact-signing/faq).
- **Who can use it:** Public Trust certificates are open to organizations in the US, Canada, the EU, the UK, Australia, New Zealand, Japan, South Korea, Singapore, Switzerland, Norway and Israel. "Individual developers must be located in the United States or Canada." [quickstart (updated 2026-10-08)](https://learn.microsoft.com/en-us/azure/artifact-signing/quickstart).
  - **An individual in France is not eligible.** A French or other EU legal entity is.
  - A GitHub organization is not a legal entity.
- **Organization validation:**
  - Asks for the legal business name, a website, an email on the organization's domain, a business identifier and address, and identity checks on a representative.
  - Supporting documents must be issued within the last 12 months.
  - It takes 1–20 business days.
  - The certificate's CN/O is the validated legal name; custom names are refused.
  - [quickstart](https://learn.microsoft.com/en-us/azure/artifact-signing/quickstart), [FAQ](https://learn.microsoft.com/en-us/azure/artifact-signing/faq).
  - A "3 years of verifiable history" rule was applied in 2024, per a Microsoft employee's Q&A answer. [Q&A 1843216](https://learn.microsoft.com/answers/a/1843216). It isn't in the current documentation; whether it still applies in 2026 is **UNVERIFIED**.
- **Certificates:** valid 72 hours and renewed daily, so timestamping is mandatory (`http://timestamp.acs.microsoft.com`). No EV is offered. [Certificate management](https://learn.microsoft.com/en-us/azure/artifact-signing/concept-certificate-management), [signing integrations](https://learn.microsoft.com/en-us/azure/artifact-signing/how-to-signing-integrations).
- **GitHub Action:** `azure/artifact-signing-action`, which runs on Windows x64 runners. [artifact-signing-action](https://github.com/azure/artifact-signing-action).
- **arm64:** no Microsoft statement found on signing arm64 PE files. Authenticode doesn't depend on the architecture (**UNVERIFIED** in the docs).
- **Linux runners:** not among Microsoft's listed integrations. Jsign supports the service as `TRUSTEDSIGNING`. [Jsign](https://ebourg.github.io/jsign/).

### CA/B Forum hardware-key rule and cloud HSMs

- Since 2023-06-01, code signing private keys must be held in FIPS 140-2 Level 2 / Common Criteria EAL4+ hardware, a cloud key service or a signing service (§6.2.7.4.1). [CA/B Forum Code Signing BRs](https://cabforum.org/working-groups/code-signing/requirements/).
- Certificates issued on or after 2026-03-01 last at most 460 days (ballot CSC-31, §6.3.2; current version v3.11.0, 2026-06-16). Same source.
- The consequence for CI: no `.pfx` file in a secret. Signing goes through a remote HSM or a signing service:
  - **SSL.com eSigner:** IV/OV plans from $20/month (20 signings) to $250/month (1,000 signings). CodeSignTool runs on Windows, macOS and Linux; the GitHub Action is `sslcom/actions-codesigner`. [pricing](https://www.ssl.com/guide/esigner-pricing-for-code-signing/), [GitHub Actions](https://www.ssl.com/how-to/cloud-code-signing-integration-with-github-actions/).
  - **DigiCert KeyLocker:** from $44/month per certificate, with 1,000 signatures per year of validity. [licensing](https://docs.digicert.com/en/digicert-keylocker/overview/licensing.html). Bun uses it.
  - **Certum Open Source Code Signing:** €49 in the SimplySign cloud or €69 on a card, capped at 5,000 signatures/month. It is issued to the individual, with the subject prefixed "Open Source Developer". [Certum shop](https://shop.certum.eu/open-source-code-signing-on-simplysign.html). SimplySign uses a mobile app for codes; whether it can sign unattended in CI is **UNVERIFIED**.
  - **Azure Key Vault Premium or Managed HSM:** holds a non-exportable RSA-HSM key under a certificate from a CA. CAs differ on whether they accept Key Vault's key attestation. [GlobalSign](https://support.globalsign.com/code-signing/code-signing/code-signing-certificate-setup-azure-key-vault), [SSL.com supported HSMs](https://www.ssl.com/guide/supported-cloud-hsms-document-signing-ev-code-signing/).
  - **Jsign:** signs from Linux against Azure Key Vault, Google Cloud KMS, eSigner, DigiCert ONE and Artifact Signing. [Jsign](https://ebourg.github.io/jsign/).

### SignPath Foundation (free for open source)

- **Whose name:** the certificate is issued to SignPath Foundation, which appears as the publisher. The keys stay in SignPath's HSM. [signpath.org](https://signpath.org/).
- **Eligibility:** OSI license without commercial dual-licensing, no proprietary components, actively maintained, already released and documented, and a one-time check of "project reputation and control". [SignPath Foundation terms](https://signpath.org/terms).
- **Policy text relevant to Goodfetch:** the terms say software "must not contain malware or potentially unwanted programs", "must not include features designed to identify or exploit security vulnerabilities or circumvent security measures of their execution environment", "must not include features that compromise the privacy or security of users", and "must not modify the user's system configuration without proper warnings". [terms](https://signpath.org/terms).
  - Projects also need an uninstaller, MFA for every team member, defined roles, manual approval of each release, and a code signing policy published on the homepage. Same source.
  - The terms don't mention anti-detect browsers or scrapers. Whether "circumvent security measures" would cover fingerprint evasion, or a service that adds WFP filters, is SignPath's call (**UNVERIFIED**).
- **GitHub Actions:** `signpath/github-action-submit-signing-request` with artifacts uploaded through `actions/upload-artifact`. Open-source projects must build every job on GitHub-hosted runners before signing. [SignPath GitHub docs](https://docs.signpath.io/trusted-build-systems/github).
- No surveyed peer uses SignPath (section 3).

### Signing the Windows service and the WFP broker

- **Kernel-mode drivers** must be signed through Microsoft's hardware dev portal, with attestation or HLK, using an EV-backed account. [Kernel-mode code signing policy](https://learn.microsoft.com/en-us/windows-hardware/drivers/install/kernel-mode-code-signing-policy--windows-vista-and-later-). Microsoft's FAQ draws the line: Partner Center for kernel mode, Artifact Signing for user mode. [Artifact Signing FAQ](https://learn.microsoft.com/en-us/azure/artifact-signing/faq).
- **User-mode WFP:** `FwpmFilterAdd0` access is controlled by tokens and security descriptors. Administrators have full access by default, and no page requires the caller to be signed. [WFP access control](https://learn.microsoft.com/en-us/windows/win32/fwp/access-control), [FwpmFilterAdd0](https://learn.microsoft.com/en-us/windows/win32/api/fwpmu/nf-fwpmu-fwpmfilteradd0).
- The service therefore has no Windows *requirement* to be signed. SAC enforcement still blocks it when unsigned (see above).
- Microsoft also advises against signing potentially unwanted apps, because the certificate itself then gains a bad reputation. [SmartScreen reputation](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/smartscreen-reputation).

### Checking a caller's signature in the service

1. Get the pipe client's process ID with `GetNamedPipeClientProcessId`. [doc](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-getnamedpipeclientprocessid)
2. Call `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)`, then `QueryFullProcessImageNameW` for the image path. [doc](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-queryfullprocessimagenamew)
3. Call `WinVerifyTrust` with `WINTRUST_ACTION_GENERIC_VERIFY_V2`. It checks the chain to a trusted root and the code-signing usage, but not *who* signed. Close the state afterwards with `WTD_STATEACTION_CLOSE`. [Verifying the signature of a PE file](https://learn.microsoft.com/en-us/windows/win32/seccrypto/example-c-program--verifying-the-signature-of-a-pe-file)
4. Match the signer certificate's subject. Either:
   - `WTHelperProvDataFromStateData` then `WTHelperGetProvSignerFromChain` [doc](https://learn.microsoft.com/en-us/windows/win32/api/wintrust/nf-wintrust-wthelperprovdatafromstatedata), or
   - `CryptQueryObject`, then `CryptMsgGetParam`, `CertFindCertificateInStore` and `CertGetNameString`. [Authenticode info](https://learn.microsoft.com/en-us/previous-versions/troubleshoot/windows/win32/get-information-authenticode-signed-executables)
5. Restrict the pipe with a DACL (by default anyone can read it), and use the logon SID to keep out other sessions. [Named pipe security](https://learn.microsoft.com/en-us/windows/win32/ipc/named-pipe-security-and-access-rights)

Caveats:
- `WINTRUST_FILE_INFO` accepts an open file handle, which narrows the gap between checking the file and using it. [WINTRUST_FILE_INFO](https://learn.microsoft.com/en-us/windows/win32/api/wintrust/ns-wintrust-wintrust_file_info)
- The following are the author's reasoning, not Microsoft documentation (**UNVERIFIED**):
  - process IDs can be reused;
  - the file at a path can be swapped between the check and its use;
  - a signed exe can still load hijacked DLLs;
  - with 72-hour Artifact Signing certificates, pin the subject, not a thumbprint.

## 3. Peer practice, from their release workflows

| Project | macOS | Windows | Source |
|---|---|---|---|
| Deno | Developer ID + hardened runtime via `rcodesign sign` on a macOS runner; no notarization step found | Artifact Signing (`Azure/artifact-signing-action`) | [ci.ts](https://github.com/denoland/deno/blob/main/.github/workflows/ci.ts) |
| Bun | Ad-hoc only, re-signed with entitlements on arm64; "x64 ships unsigned"; no Developer ID | DigiCert KeyLocker (`smctl` + signtool) in Buildkite | [shims.ts](https://github.com/oven-sh/bun/blob/main/scripts/build/shims.ts), [sign-windows-artifacts.ps1](https://github.com/oven-sh/bun/blob/main/.buildkite/scripts/sign-windows-artifacts.ps1) |
| uv / ruff (Astral) | Signed and notarized with `rcodesign` on Ubuntu, key in Azure Key Vault via PKCS#11 | Artifact Signing | [uv sign-release-binaries.yml](https://github.com/astral-sh/uv/blob/main/.github/workflows/sign-release-binaries.yml), [sign-macos](https://github.com/astral-sh/github-actions/blob/main/sign-macos/action.yml), [sign-windows](https://github.com/astral-sh/github-actions/blob/main/sign-windows/action.yml), [ruff](https://github.com/astral-sh/ruff/blob/main/.github/workflows/sign-release-binaries.yml) |
| Biome | No signing step found (codesign/notarize searches empty) | No signing step found | [release_cli.yml](https://github.com/biomejs/biome/blob/main/.github/workflows/release_cli.yml) |
| Playwright | Chromium comes from Chrome for Testing ([registry](https://github.com/microsoft/playwright/blob/main/packages/playwright-core/src/server/registry/index.ts)); a maintainer said its Chromium is unsigned and pointed to `channel: 'chrome'` ([#10642](https://github.com/microsoft/playwright/issues/10642)). Firefox/WebKit signing **UNVERIFIED**; the build scripts are no longer in the repo | **UNVERIFIED** | [#5193](https://github.com/microsoft/playwright/issues/5193) (driver failed notarization in a host app) |
| Chrome for Testing | Not Developer ID signed and not notarized; the main Mach-O has only the linker's ad-hoc signature ([#227](https://github.com/GoogleChromeLabs/chrome-for-testing/issues/227)); README says to run `xattr -cr` | Unsigned, "no current plans to change this" ([#175](https://github.com/GoogleChromeLabs/chrome-for-testing/issues/175)) | [README](https://github.com/GoogleChromeLabs/chrome-for-testing/blob/main/README.md#macos-says-the-app-is-damaged-what-now) |
| ungoogled-chromium | Signed and notarized with an individual Developer ID ("Qian Qian"), paid by sponsors and promised until 2027-10-14; uses `codesign`, `notarytool` and `stapler` | No signing step in the Windows workflows | [macOS README](https://github.com/ungoogled-software/ungoogled-chromium-macos/blob/main/README.md), [sign_and_package_app.sh](https://github.com/ungoogled-software/ungoogled-chromium-macos/blob/main/sign_and_package_app.sh), [issue #184](https://github.com/ungoogled-software/ungoogled-chromium-macos/issues/184), [Windows workflows](https://github.com/ungoogled-software/ungoogled-chromium-windows/tree/main/.github/workflows) |
| fingerprint-chromium | **UNVERIFIED** (no public build pipeline; recent releases mostly Windows/Linux) | **UNVERIFIED** | [repo](https://github.com/adryfish/fingerprint-chromium), [releases](https://github.com/adryfish/fingerprint-chromium/releases) |
| Camoufox | No signing step found (all builds run on Ubuntu) | No signing step found | [release.yml](https://github.com/daijro/camoufox/blob/main/.github/workflows/release.yml) |
| Codex CLI (OpenAI) | Signed and notarized with `rcodesign` on Ubuntu, key in Azure Key Vault via PKCS#11, protected `codesigning` environment | Azure Trusted/Artifact Signing | [rust-release.yml](https://github.com/openai/codex/blob/main/.github/workflows/rust-release.yml), [windows-code-sign](https://github.com/openai/codex/blob/main/.github/actions/windows-code-sign/action.yml) |
| Claude Code | Docs: "signed by 'Anthropic PBC' and notarized by Apple" | Docs: signed by "Anthropic, PBC" | [setup docs](https://code.claude.com/docs/en/setup#platform-code-signatures); no public workflow, so the tooling is **UNVERIFIED** |
| GitHub CLI | `codesign` with a Developer ID `.p12` on `macos-latest`; `notarytool` with an App Store Connect API key; zips and a `.pkg` | Azure Trusted Signing (signtool + dlib, account `GitHubInc`) for exe and msi | [deployment.yml](https://github.com/cli/cli/blob/trunk/.github/workflows/deployment.yml) |
| Tailscale | The GUI apps are closed source; notarization is not stated on the KB page (**UNVERIFIED**). No prebuilt open-source macOS CLI: `go install` or the Homebrew formula | MSI signing **UNVERIFIED** from official sources | [kb/1065](https://tailscale.com/kb/1065/macos-variants), [wiki](https://github.com/tailscale/tailscale/wiki/Tailscaled-on-macOS) |

Patterns:
- **Windows:** Artifact Signing is the most common choice (Deno, uv, ruff, Codex, GitHub CLI). Bun uses DigiCert KeyLocker. None of the projects surveyed uses SignPath.
- **macOS:** notarization from Linux with `rcodesign` and a Key Vault HSM key (Astral, Codex), or `codesign` + `notarytool` on a macOS runner (GitHub CLI, ungoogled-chromium).
- **Browsers:** they are the least signed group (Chrome for Testing, Playwright's Chromium, Camoufox, ungoogled-chromium on Windows). ungoogled-chromium on macOS is the exception.

What users must do:
- **ungoogled-chromium, macOS:** nothing now that builds are notarized. The README suggests checking with `spctl -a -vvv -t install` for `source=Notarized Developer ID`. `codesign --sign -` appears only in the ad-hoc path for self-builds. [README](https://github.com/ungoogled-software/ungoogled-chromium-macos/blob/main/README.md).
- **ungoogled-chromium, Windows:** the binaries are unsigned. That users must click through SmartScreen is inferred; the README doesn't say (**UNVERIFIED**).
- **Camoufox, macOS:**
  - There are no official docs.
  - A maintainer gives `xattr -d com.apple.quarantine <Camoufox.app>` as the fix for browser-downloaded copies. [#480](https://github.com/daijro/camoufox/issues/480), [#464](https://github.com/daijro/camoufox/issues/464).
  - The Python package downloads and unzips the browser itself with no quarantine handling, and needs none, since its downloads aren't quarantined. [pkgman.py](https://github.com/daijro/camoufox/blob/main/python/src/camoufox/pkgman.py).
- **Camoufox, Windows:** unsigned; no official SmartScreen guidance found (**UNVERIFIED**).

## 4. Linux

- **IMA/EVM:**
  - IMA measures files, appraises their hash or signature (`security.ima`) and can deny access; EVM protects that metadata. [IMA concepts](https://ima-doc.readthedocs.io/en/latest/ima-concepts.html).
  - IMA acts only when a policy is loaded: built-in policies are opt-in through `ima_policy=`, and `appraise_tcb` appraises all root-owned files. Without a policy, "the default policy is to not appraise anything". [kernel parameters](https://www.kernel.org/doc/html/latest/admin-guide/kernel-parameters.html), [IMA configuration](https://ima-doc.readthedocs.io/en/latest/ima-configuration.html), [linux-ima wiki](https://sourceforge.net/p/linux-ima/wiki/Home/).
  - No general-purpose distro enforcing appraisal by default was found (**UNVERIFIED** as a blanket statement).
- **Fedora and RHEL:**
  - Fedora 37 added per-file IMA signatures inside RPMs, with no default policy. [Fedora change](https://fedoraproject.org/wiki/Changes/Signed_RPM_Contents)
  - RHEL 9+ signs every package file, and appraisal is turned on manually. [RHEL 10 kernel integrity](https://docs.redhat.com/en/documentation/red_hat_enterprise_linux/10/html/managing_monitoring_and_updating_the_kernel/enhancing-security-with-the-kernel-integrity-subsystem)
  - Third-party binaries would need the administrator's own key, signing with `evmctl`. [linux-ima wiki](https://sourceforge.net/p/linux-ima/wiki/Home/)
- **fs-verity** gives per-file Merkle verification. It has no mandatory authentication scheme, and its built-in signature check is optional. [fs-verity](https://docs.kernel.org/filesystems/fsverity.html).
- **Secure Boot and lockdown** cover the shim, the bootloader, the kernel, kernel modules and kexec images, not userspace executables. [Debian SecureBoot](https://wiki.debian.org/SecureBoot), [module signing](https://docs.kernel.org/admin-guide/module-signing.html), [kernel_lockdown(7)](https://www.mankier.com/7/kernel_lockdown).
- **fapolicyd** (RHEL application allowlisting):
  - Trusts the rpm database by default, and can check size, IMA hash or SHA-256. [fapolicyd.conf(5)](https://github.com/linux-application-whitelisting/fapolicyd/blob/main/doc/fapolicyd.conf.5)
  - A binary installed outside a package must be added to `fapolicyd.trust`, or it is denied when the allowlist is enforced. [fapolicyd](https://github.com/linux-application-whitelisting/fapolicyd)
- **SELinux and AppArmor** confine programs by label or path policy, not by signature (no source fetched; **UNVERIFIED** here).
- **Snap and Flatpak:** snaps carry store-signed assertions [snap assertions](https://snapcraft.io/docs/assertions); Flatpak repository commits are GPG-signed [Flatpak builder](https://docs.flatpak.org/en/latest/flatpak-builder.html).
- **Setuid helpers:** no kernel or distro rule requires a setuid binary to be signed. Chromium's old SUID sandbox needed only `root:root` ownership and mode 4755. [suid_sandbox_development.md](https://chromium.googlesource.com/chromium/src/+/HEAD/docs/linux/suid_sandbox_development.md). Where IMA appraisal or fapolicyd is enforced, an unsigned root-owned helper is denied until the administrator trusts it.
- **Distribution norms:** the distribution channel is what gets signed, not the executable:
  - **GPG-signed apt repositories:** Debian's `Release` file is signed with OpenPGP, and `apt-key` was removed in Trixie in favour of `signed-by` keyrings. [SecureApt](https://wiki.debian.org/SecureApt). GitHub CLI [install_linux.md](https://github.com/cli/cli/blob/trunk/docs/install_linux.md), Docker [Debian install](https://docs.docker.com/engine/install/debian/) and Tailscale [pkgs](https://pkgs.tailscale.com/stable/) all publish signed apt repos.
  - **dnf `gpgcheck`:** [DNF conf](https://dnf.readthedocs.io/en/latest/conf_ref.html).
  - **Sigstore** `cosign verify` / `verify-blob` against an identity and an OIDC issuer: [cosign](https://docs.sigstore.dev/cosign/verifying/verify/).
  - **GitHub artifact attestations** (SLSA v1.0 Build Level 2, `gh attestation verify`): [docs](https://docs.github.com/en/actions/concepts/security/artifact-attestations).
  - **npm provenance:** [npm](https://docs.npmjs.com/generating-provenance-statements).
  - **Homebrew bottle attestation verification:** [Homebrew::Attestation](https://docs.brew.sh/rubydoc/Homebrew/Attestation.html); whether it is on by default is **UNVERIFIED**.

## Implications for Goodfetch

- **The CLI on macOS:** no Goodfetch install channel (npm, pip, crates.io, the Homebrew formula, the `curl | sh` installer) sets quarantine. On arm64 the linker's ad-hoc signature is enough to run. Gatekeeper only matters when a user downloads the archive from GitHub Releases in a browser, or if Goodfetch ships a cask. Notarizing the bare CLI inside a zip covers that case, but online only, since a ticket can't be stapled to a bare Mach-O.
- **Homebrew:** a formula in Goodfetch's own tap is outside the cask policy. A cask in homebrew/cask would have to pass Gatekeeper, and users can no longer bypass that with `--no-quarantine`. Since 6.0.0, users must explicitly trust third-party taps.
- **The Chromium download:** the engine's own HTTP client doesn't set quarantine on macOS or Mark-of-the-Web on Windows, so Gatekeeper and SmartScreen don't apply to the cached browser. Two exceptions:
  - Smart App Control checks every executable whatever its origin, so an unsigned Chromium (and unsigned `goodfetch.exe` and broker) is blocked wherever SAC enforces.
  - A browser-downloaded archive copied in through the offline env var keeps its quarantine.
  - Any post-link patching on macOS requires re-signing, at least ad-hoc, for arm64.
- **Notarizing Chromium** needs the hardened runtime with `allow-jit` on the renderer and GPU helpers, copying Chrome's `parts.py` scheme, as ungoogled-chromium does successfully.
- **Apple account:**
  - An individual enrollment puts the owner's legal name on every Developer ID signature.
  - Showing "Goodfetch" requires a legal entity with a D-U-N-S number and its own domain; a GitHub org doesn't qualify.
  - The ADPLA lets Apple revoke at its discretion. No anti-detect revocation precedent was found.
- **Windows signing paths:**
  - Artifact Signing is the peer default and the cheapest at $9.99/month, but it is closed to an individual in France. It needs a French/EU legal entity, possibly with a history requirement (unverified).
  - Without an entity, the options are SignPath Foundation (publisher "SignPath Foundation", subject to policy text that may exclude "circumvent security measures") or an OV certificate on a cloud HSM (Certum open-source certificate in the individual's name, SSL.com eSigner, DigiCert KeyLocker), signed from CI through Jsign or the vendor tool.
  - EV buys nothing for SmartScreen since 2024.
- **The broker service:** Windows doesn't require it to be signed in user mode, and WFP gates on admin rights. Signing it still matters for SAC, and it lets the service authenticate its caller (`WinVerifyTrust` plus a subject match, behind a DACL-restricted pipe). That check is only meaningful if `goodfetch.exe` is signed.
- **Linux:** platform signing doesn't gate a root-installed helper unless an administrator enforces IMA appraisal or fapolicyd. The existing Sigstore signatures and GitHub attestations match peer practice. A GPG-signed apt/rpm repository would be the next norm if Goodfetch publishes system packages. Documenting how to add the helper to fapolicyd or IMA trust covers hardened hosts.
- **macOS from Linux CI:** `rcodesign` with an App Store Connect API key and an HSM-held key (as Astral and OpenAI do) fits the existing single-build GitHub Actions `xtask release` without a macOS signing runner. Its last tagged release is from 2024.

## Open uncertainties

- Whether npm and pip ever set quarantine on macOS, and whether a terminal hosted by a quarantine-enabled app propagates it.
- Whether Gatekeeper enforcement changed in macOS 26.
- The current EUR price of the Apple Developer Program.
- Whether Apple accepts a loi 1901 association, and whether France is eligible for the fee waiver.
- Whether the 3-year organization-age rule still applies to Artifact Signing.
- Whether Microsoft documents arm64 support for Artifact Signing.
- Whether SAC still requires a clean install the first time, and how many machines have it.
- Whether SignPath Foundation would accept a stealth browser or a WFP-filtering tool.
- Whether Certum SimplySign can sign unattended in CI.
- How Playwright's Firefox and WebKit builds, Tailscale's MSI and fingerprint-chromium are signed.
- Whether Homebrew actually applied the September 2026 cask disables.
