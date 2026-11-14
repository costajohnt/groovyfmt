# Releasing and publishing

## A release

1. Bump `version` in `Cargo.toml` (and `Cargo.lock`) and in the VS Code extension (`npm version <version> --no-git-tag-version` in `editors/vscode`, which updates `package.json` and `package-lock.json`; CI and the tag check fail when the two differ), add a heading to `editors/vscode/CHANGELOG.md` if the extension itself changed, and to `editors/jetbrains/CHANGELOG.md` if the JetBrains plugin changed (the plugin takes its version from `Cargo.toml`, and its Marketplace change notes are that file's first section), add a `## <version> (<date>)` heading to `CHANGELOG.md`, and update the version pinned in the examples (`costajohnt/groovyfmt@v<version>`, `rev: v<version>`, `version: <version>`, `version = '<version>'`) in `README.md` and `docs/ci.md`; `grep -rnF '<old version>' README.md docs/ci.md` finds them. Merge to `main`.
2. Run `gh workflow run ci.yml --ref main` and wait for it to pass. Pull requests and pushes skip the two macOS release builds and the macOS action and npm jobs (hosted macOS runners are scarce); this run has them, so a macOS break shows up before the tag does.
3. Tag the merge commit `v<version>` and push the tag.

`release.yml` then checks the tag (`check`), runs all of `ci.yml` on it (`ci`), and creates the GitHub release with the five archives and their `.sha256` files (`release`): a `.tar.gz` for each Linux and macOS target and a `.zip` for Windows, plus the VS Code extension as `groovyfmt-<version>.vsix`. After that, the `jetbrains` job tests the JetBrains plugin, packages it and attaches `groovyfmt-jetbrains-<version>.zip` to the release; `release` doesn't wait for it, so a failure there leaves the release without the zip, and rerunning that job attaches it. The pre-commit hooks (`.pre-commit-hooks.yaml`) and the GitHub Action (`action.yml`) need nothing more: they are used straight from the tag.

Seven more jobs publish to registries. Each is off until a repository variable switches it on, and each runs only after `release` succeeds (`publish-jetbrains` after the JetBrains build), on the tag push itself. Rerunning a whole release workflow fails at `release` because the GitHub release already exists, so switch a channel on before pushing the tag you want it to publish.

| Variable | Job | Publishes |
|---|---|---|
| `PUBLISH_CRATE` = `true` | `publish-crate` | `groovyfmt` to crates.io, through trusted publishing |
| `PUBLISH_NPM` = `true` | `publish-npm` | `groovyfmt`, `@groovyfmt/{darwin,linux}-{arm64,x64}` and `@groovyfmt/win32-x64` to npm, with provenance |
| `PUBLISH_IMAGE` = `true` | `publish-image` | `ghcr.io/costajohnt/groovyfmt:<version>` and `:latest` for linux/amd64 and linux/arm64 |
| `PUBLISH_GRADLE_PLUGIN` = `true` | `publish-gradle-plugin` | the Gradle plugin `io.github.costajohnt.groovyfmt` to the Gradle Plugin Portal |
| `PUBLISH_VSCODE` = `true` | `publish-vscode` | the VS Code extension `groovyfmt.groovyfmt` to the Visual Studio Marketplace |
| `PUBLISH_OPENVSX` = `true` | `publish-openvsx` | the same extension to Open VSX (VSCodium, Cursor and other VS Code builds) |
| `PUBLISH_JETBRAINS` = `true` | `publish-jetbrains` | the JetBrains plugin `groovyfmt` (vendor `groovyfmt`) to JetBrains Marketplace |

Set one with `gh variable set PUBLISH_NPM --body true --repo costajohnt/groovyfmt`, and unset it the same way with `--body false` or `gh variable delete`. A version with a `-` (a prerelease) is published under the npm dist-tag `next` and gets no `latest` image tag.

## npm

`npm/build.mjs <version> <archives-dir>` builds the packages into `npm/dist/` from the release archives, checking each against its `.sha256`: one package per platform holding the binary, with `os` and `cpu` set so npm installs only the matching one, and the `groovyfmt` package with a small Node launcher and those packages as optional dependencies. CI builds them from the latest release, packs them, installs the tarballs and runs `groovyfmt --version` on Linux and macOS. The Windows package is built from the `.zip` (unpacked with Node's own zlib), so until a release has the Windows asset, a by-hand CI run fails at its npm job.

`publish-npm` publishes with npm trusted publishing (OIDC): there is no npm token secret, and provenance is attached automatically. That needs, once:

1. The repository public. npm does not attach provenance for private repositories, and the job passes `--provenance`.
2. The `@groovyfmt` scope: on npmjs.com, create the organization `groovyfmt` (free for public packages) with your account as owner. Done on 2026-11-11: the organization exists and 0.9.1 of all six packages was published by hand.
3. A trusted publisher on each of the six packages (`groovyfmt`, `@groovyfmt/darwin-arm64`, `@groovyfmt/darwin-x64`, `@groovyfmt/linux-arm64`, `@groovyfmt/linux-x64`, `@groovyfmt/win32-x64`): in the package's Settings, Trusted Publisher, GitHub Actions, with organization or user `costajohnt`, repository `groovyfmt`, workflow filename `release.yml`, and no environment. Then, under publishing access, require two-factor authentication and disallow tokens.

   The trusted publisher form lives on a package's settings page, so a package may have to exist first. If so, publish the first version of each package by hand (`npm login`, then `node npm/build.mjs v<version> <dir>` on archives fetched with `gh release download v<version> --repo costajohnt/groovyfmt --pattern 'groovyfmt-*'`, then `npm publish ./npm/dist/<package> --access public` for the five platform packages and `groovyfmt` last), configure the trusted publishers, and let CI publish every later version.
4. `gh variable set PUBLISH_NPM --body true --repo costajohnt/groovyfmt`.

`package.json` names `https://github.com/costajohnt/groovyfmt` as the repository, which trusted publishing requires to match the publishing repository exactly. A rerun skips versions already on the registry.

## Windows

The Windows build is cross-compiled from Linux, so no workflow needs a Windows runner. `.cargo/config.toml` sets the `x86_64-pc-windows-gnu` linker to mingw-w64's `x86_64-w64-mingw32-gcc` and links the C runtime statically, so `groovyfmt.exe` imports only DLLs that are part of Windows (`KERNEL32`, `ntdll`, `msvcrt`, `bcryptprimitives` and `api-ms-win-core-synch-l1-2-0`, as of this writing). In `ci.yml`'s `build` job it is built on `ubuntu-latest` with `gcc-mingw-w64-x86-64` from apt, its imports are checked with `x86_64-w64-mingw32-objdump -p`, it is smoke-tested under Wine (`wine64` from apt), and it is packaged as `groovyfmt-<tag>-x86_64-pc-windows-gnu.zip` (`groovyfmt.exe`, README, LICENSE) with a `.sha256`.

To build and test it locally (Debian or Ubuntu: `gcc-mingw-w64-x86-64` and `wine64`, plus `rustup target add x86_64-pc-windows-gnu`):

```sh
cargo build --release --locked --target x86_64-pc-windows-gnu
CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUNNER=$PWD/scripts/wine-run.sh cargo test --locked --target x86_64-pc-windows-gnu
```

`scripts/wine-run.sh` runs each test binary, and through it `groovyfmt.exe`, under Wine with a UTF-8 locale and the prefix `~/.wine-gf`; `scripts/ci-local.sh` does all of this when the tools are there. Wine maps `/` to the drive `Z:`, so Linux paths work as arguments. The test binaries also run on a Windows machine as they are: copy them with `groovyfmt.exe` and `tests/fixtures`, and set `GROOVYFMT_TEST_BIN` and `GROOVYFMT_TEST_FIXTURES` to those copies (the paths cargo bakes in are Linux ones). That is how the results in [Windows](windows.md) were checked on Windows Server 2022.

## Docker image

`packaging/docker/context.sh <archives-dir> <out-dir>` checks the two linux-musl archives against their `.sha256` and unpacks the binaries into a build context for the `Dockerfile` (a `scratch` image with only the binary and the license). CI builds it for both platforms on every pull request and runs it on amd64 and arm64.

1. `gh variable set PUBLISH_IMAGE --body true --repo costajohnt/groovyfmt`. The job logs in to ghcr.io with the workflow's own token (`packages: write`).
2. After the first push, the package `groovyfmt` appears under the account's Packages, linked to the repository by the image's `org.opencontainers.image.source` label. A package first pushed from a private repository is private: in its Package settings, change the visibility to public.

## Gradle plugin

`gradle-plugin/` is a Gradle build of its own (Java, no runtime dependencies). Its version is read from `Cargo.toml`, so a release tag publishes the plugin at the tag's version, and by default it downloads that release. `./gradlew check` there runs the unit tests and the TestKit tests on the wrapper's Gradle and on the oldest supported one (`minGradle` in `build.gradle`); `scripts/ci-local.sh` runs it when a JDK 17+ is present. The wrapper jar is Gradle's own (its SHA-256 matches `https://services.gradle.org/distributions/gradle-<version>-wrapper.jar.sha256`) and `distributionSha256Sum` pins the distribution; to upgrade, run `./gradlew wrapper --gradle-version <v> --gradle-distribution-sha256-sum <sum>` with the sum from `https://services.gradle.org/distributions/gradle-<v>-bin.zip.sha256`.

`publish-gradle-plugin` runs `./gradlew check publishPlugins` (`com.gradle.plugin-publish`). That needs, once:

1. The repository public. The plugin downloads release assets, and once published anyone can apply it; while the repository is private, only users with a token can.
2. A Gradle Plugin Portal account (https://plugins.gradle.org, sign in with GitHub as `costajohnt`). Sign in with the GitHub account: an `io.github.costajohnt.*` id is approved automatically when the account is linked to that GitHub user, and goes to manual review otherwise. The id was unclaimed when this was written.
3. An API key: on the portal, your profile, API Keys. Store the two values as repository secrets: `gh secret set GRADLE_PUBLISH_KEY --repo costajohnt/groovyfmt` and `gh secret set GRADLE_PUBLISH_SECRET --repo costajohnt/groovyfmt` (each prompts for the value).
4. `gh variable set PUBLISH_GRADLE_PLUGIN --body true --repo costajohnt/groovyfmt`.

The portal refuses a version it already has, so a rerun of an already published version fails at `publishPlugins`. To publish by hand instead: `cd gradle-plugin && GRADLE_PUBLISH_KEY=... GRADLE_PUBLISH_SECRET=... ./gradlew publishPlugins` from the tagged commit (`--validate-only` checks everything without uploading).

The two secrets are set only for the job's one step, but that step runs the whole Gradle build, its plugins and the tests with them in the environment, so a compromised build dependency could read them. The follow-up is to build and test in a job without secrets and publish the prebuilt artifact in a separate step, as `publish-jetbrains` does.

## VS Code extension

`editors/vscode/` is the extension (TypeScript, bundled with esbuild into one file, so the `.vsix` holds no `node_modules`). Its version must equal the `Cargo.toml` version: with no groovyfmt installed, it downloads the release of its own version. `ci.yml`'s `vscode` job lints it, runs its unit tests and two integration tests in VS Code (against the commit's own groovyfmt, and in Restricted Mode, where nothing the workspace plants may run), and packages a `.vsix` to try out. That `.vsix` carries no checksums, so it refuses to download groovyfmt.

The release `.vsix` comes from `release.yml`'s `vscode` job, which runs after `ci` has built the five archives in the same run. It hashes those archives (`npm run checksums -- <version> <archives-dir>`, which also checks each against its `.sha256`) into `dist/checksums.json`, packages the `.vsix` with it, and uploads it as the `vscode-extension` artifact that `release` attaches to the GitHub release. The extension downloads groovyfmt only when the archive matches the checksum it carries, and never trusts a checksum fetched from the release, so rewriting a release's assets can't change what it runs. The GitHub Action still checks a download against the release's own `.sha256` file ([verifying release downloads](#verifying-release-downloads)). `scripts/ci-local.sh` runs the lint, the unit tests and the packaging when Node.js 22 or later is present.

Both publish jobs publish that same `.vsix`, skip a version the registry already has (so a rerun finishes the job), and skip prerelease versions, which the Marketplace doesn't accept.

### Visual Studio Marketplace

`publish-vscode` publishes with Marketplace trusted publishing (OIDC, `vsce publish --oidc`): no token secret. Azure DevOps retires global personal access tokens on 2026-12-01, so a token-based setup would stop working then. That needs, once:

1. A publisher with the id `groovyfmt`, the `publisher` in `editors/vscode/package.json`: sign in at https://marketplace.visualstudio.com/manage with a Microsoft account, Create publisher, ID `groovyfmt`. The id can't be changed later. Done on 2026-11-13: the publisher exists.
2. A trusted publishing policy: on that page, open the publisher, Trusted Publishing, Create, with repository owner `costajohnt`, repository `groovyfmt`, workflow `release.yml`, and no environment. Done on 2026-11-13: the policy is active.
3. `gh variable set PUBLISH_VSCODE --body true --repo costajohnt/groovyfmt`.

If a repository secret `VSCE_PAT` exists, the job uses it instead of OIDC. It would be an Azure DevOps personal access token with the Organization "All accessible organizations" and the scope Marketplace, Manage (`gh secret set VSCE_PAT --repo costajohnt/groovyfmt`). Leave it unset to use trusted publishing.

To publish by hand: `npx vsce login groovyfmt`, then `npx vsce publish --packagePath groovyfmt-<version>.vsix` in `editors/vscode` with the `.vsix` from the release.

### Open VSX

`publish-openvsx` runs `ovsx publish` with the secret `OVSX_PAT`. That needs, once:

1. An account at https://open-vsx.org (sign in with GitHub as `costajohnt`), with the Eclipse Foundation account linked and the Open VSX Publisher Agreement signed in its settings.
2. An access token: Settings, Access Tokens, Generate New Token. Store it with `gh secret set OVSX_PAT --repo costajohnt/groovyfmt`.
3. The namespace `groovyfmt`, which must equal the `publisher` in `editors/vscode/package.json`: `npx ovsx create-namespace groovyfmt -p <token>` (run in `editors/vscode`, where `ovsx` is a dev dependency). `npx ovsx verify-pat groovyfmt -p <token>` checks the token can publish to it.
4. `gh variable set PUBLISH_OPENVSX --body true --repo costajohnt/groovyfmt`.

A new namespace's extensions show as unverified until the namespace is claimed; see the Open VSX wiki's "Namespace access" page to claim it.

## JetBrains plugin

`editors/jetbrains/` is the plugin (Kotlin, built with the IntelliJ Platform Gradle Plugin 2.x and JDK 21). It depends on Red Hat's LSP4IJ plugin for the language server client and runs in IDEs based on IntelliJ Platform 2024.2 or later. Its version is the `Cargo.toml` version, read by its build, and with no groovyfmt installed it downloads the release of that version. `ci.yml`'s `jetbrains` job runs its unit tests and its integration tests in IntelliJ IDEA Community 2024.2 (against the commit's own groovyfmt: diagnostics and formatting, an untrusted project whose planted binaries must not run, and LSP4IJ's user-defined server), the Plugin Verifier against IntelliJ IDEA Community 2024.2 and 2025.2, IntelliJ IDEA 2026.3 and Android Studio 2026.2, and builds a zip to try out. That zip carries no checksums, so it refuses to download groovyfmt. `scripts/ci-local.sh` runs the tests when a JDK 21 or later is the current Java or is named by `JDK21_HOME` (the Gradle plugin's oldest-Gradle tests need the current Java to be older than 21).

The release zip comes from `release.yml`'s `jetbrains` job, which runs before `release`. It builds the plugin with `-PgroovyfmtArchives=<dir>` pointing at the five archives built in the same run, which hashes them (checking each against its `.sha256`) into `groovyfmt-checksums.properties` inside the plugin, runs the tests against the release's own Linux binary, and uploads the zip as the `jetbrains-plugin` artifact. That job runs Gradle, its plugins and dependencies and an IDE, so its token is read-only. `release`, which checks nothing out and runs only `gh`, attaches `groovyfmt-jetbrains-<version>.zip` when it creates the GitHub release, since no asset can be added to an immutable release once it is published (v0.14.0's release lacks the zip for that reason). Like the VS Code extension, the plugin never trusts a checksum fetched from the release.

The build verifies every dependency it downloads against `editors/jetbrains/gradle/verification-metadata.xml` (SHA-256), the IDEs the IntelliJ Platform Gradle Plugin fetches and LSP4IJ included. After changing a dependency or an IDE version, regenerate it with `./gradlew --no-scan --write-verification-metadata sha256 check buildPlugin verifyPlugin` (with `GROOVYFMT_BIN` set) and review the diff.

`publish-jetbrains` publishes the very zip the `jetbrains` job built and tested: no checkout and no Gradle. When the signing secrets exist, it signs the zip with JetBrains' Marketplace ZIP Signer CLI (the jar pinned by version and SHA-256 in the workflow), then uploads it with the Marketplace upload API (`POST https://plugins.jetbrains.com/api/updates/upload` with the plugin's `xmlId`). Each secret is set only in the step that uses it, and the token goes to `curl` in a header file, never on a command line. A prerelease version isn't published. That needs, once:

1. A JetBrains Account, signed in at https://plugins.jetbrains.com, and an organization vendor profile named `groovyfmt` created there, as the VS Code extension's publisher is `groovyfmt`. `plugin.xml` names the vendor `groovyfmt` and gives the plugin the id `groovyfmt`; Marketplace keeps a plugin's id for good, so it can't be changed after the first upload.
2. The first upload by hand: JetBrains Marketplace only accepts a plugin's first version through its website. Take `groovyfmt-jetbrains-<version>.zip` from a release, choose Upload plugin on https://plugins.jetbrains.com, pick the vendor, the license (MIT, https://github.com/costajohnt/groovyfmt/blob/main/LICENSE) and the tags, and upload. JetBrains reviews a new plugin before it is listed.
3. A token: on https://plugins.jetbrains.com/author/me/tokens, Generate Token, then `gh secret set JETBRAINS_MARKETPLACE_TOKEN --repo costajohnt/groovyfmt`.
4. Signing, optional: Marketplace accepts an unsigned plugin, and the IDE then warns when it is installed. To sign, create a key and certificate as JetBrains' plugin signing guide describes (`openssl genpkey` with `-aes-256-cbc`, `openssl rsa`, then `openssl req -new -x509`), and store them with `gh secret set JETBRAINS_PRIVATE_KEY`, `gh secret set JETBRAINS_PRIVATE_KEY_PASSWORD` and `gh secret set JETBRAINS_CERTIFICATE_CHAIN` (each with `--repo costajohnt/groovyfmt`, the PEM contents as the value). The job signs whenever the key and the chain are set. Signing the first upload too keeps every version signed.
5. `gh variable set PUBLISH_JETBRAINS --body true --repo costajohnt/groovyfmt`.

To publish by hand instead, upload the release's zip on the plugin's Marketplace page, or with `curl --header "Authorization: Bearer <token>" -F xmlId=groovyfmt -F file=@groovyfmt-jetbrains-<version>.zip https://plugins.jetbrains.com/api/updates/upload`. Marketplace refuses a version it already has.

## Verifying release downloads

The GitHub Action (`action.yml`) downloads the archive for the runner and its `.sha256` file from the same GitHub release and compares the two. That catches a truncated or corrupted download. On its own it does not catch a replaced release: anyone who can rewrite the release's assets can replace the archive and its `.sha256` together. The VS Code extension and the JetBrains plugin are safe from this because they carry the hashes in the `.vsix` and the plugin zip (see above). The action is used straight from a tag, and a version's hashes exist only after its tag is built, so the tagged tree can't carry them.

The hardening is GitHub's [immutable releases](https://docs.github.com/en/code-security/supply-chain-security/understanding-your-software-supply-chain/immutable-releases), with the action verifying each download from an immutable release with `gh release verify-asset`. Once a release is published with immutability on, its tag can't be moved or deleted and its assets can't be added, replaced or deleted. A deleted immutable release's tag can't be reused either. GitHub also signs a release attestation recording the tag, the commit and the asset digests, which [`gh release verify-asset <tag> <file>`](https://cli.github.com/manual/gh_release_verify-asset) checks a downloaded file against.

`release.yml` creates the release with every asset in one `gh release create <tag> <assets>`, the JetBrains plugin zip included, and when assets are attached [`gh release create`](https://cli.github.com/manual/gh_release_create) creates a draft, uploads each asset and then publishes, which is the order GitHub recommends for immutable releases. What changes once the setting is on:

- Assets can't be added to or replaced on a published release, so a broken asset means a new version. A rerun of the release workflow already fails at `release` for an existing release.
- Immutability applies only to releases published after the setting is turned on. Existing releases stay mutable unless republished, and turning the setting off later leaves the releases made while it was on immutable.

The steps, for the owner:

1. Done 2026-11-13: in the repository's Settings, General, Releases, select "Enable release immutability" ([GitHub's guide](https://docs.github.com/en/code-security/how-tos/secure-your-supply-chain/establish-provenance-and-integrity/prevent-release-changes)).
2. Done with v0.14.0: push the next release tag as usual, then check that `gh api repos/costajohnt/groovyfmt/releases/tags/v<version> --jq .immutable` shows `true` and that `gh release verify v<version> --repo costajohnt/groovyfmt` passes.
3. Done: after the download and the `.sha256` comparison, `action.yml` runs `gh release verify-asset "$tag" "$dir/$archive" --repo "$repo"` when `gh api "repos/$repo/releases/tags/$tag" --jq .immutable` is `true`, and fails if it does. Older releases, which are mutable and have no release attestation, get only the `.sha256` comparison, which still catches a truncated download. A tag's release that is immutable stays so, so this check can't be skipped by rewriting it. `gh release verify-asset` arrived in gh 2.81.0 (hidden before that); on a runner with an older gh an immutable release fails to install with an error naming that version, rather than installing unverified, since every immutable release can be verified. The attestation is public for a public repository, so the default token needs no extra permission. CI's `action` job installs the latest release, which is immutable, and v0.13.1, which is not.

Artifact attestations ([`actions/attest`](https://docs.github.com/en/actions/how-tos/secure-your-work/use-artifact-attestations/use-artifact-attestations)) would go further and tie each archive to the workflow run that built it, with `gh attestation verify <file> --repo costajohnt/groovyfmt --signer-workflow costajohnt/groovyfmt/.github/workflows/release.yml` in the action. That is a change to how releases are produced. `release.yml`'s `release` job would get the permissions `id-token: write` and `attestations: write` and, before `gh release create`, an `actions/attest` step (pinned by commit like the other actions) with `subject-path` naming the five archives. The risks are a new action running with an OIDC token in the job that holds `contents: write`, a release that fails if the attestation service does, and an action that has to tell a release made before the change, which has no attestation, from a replaced asset, which has none either (by version, for example). Immutable releases give the action the protection it needs without changing the workflow, so attestations are left for later.

## Homebrew

`packaging/homebrew/render.sh <version> <dir>` prints a formula for the release from the four `.sha256` files in `<dir>`. It installs the prebuilt binary for the machine (macOS or Linux, arm64 or x86_64); there are no bottles to build. CI renders it for the latest release and syntax-checks it with `ruby -c`. Brew itself can't install it until the release downloads are public.

To create the tap, once the repository is public:

```sh
gh repo create costajohnt/homebrew-tap --public
gh repo clone costajohnt/homebrew-tap && mkdir -p homebrew-tap/Formula
gh release download v<version> --repo costajohnt/groovyfmt --pattern '*.sha256' --dir /tmp/sha
packaging/homebrew/render.sh v<version> /tmp/sha > homebrew-tap/Formula/groovyfmt.rb
# commit and push homebrew-tap, then:
brew install costajohnt/tap/groovyfmt && brew test groovyfmt
```

For each later release, render the formula again and commit it to the tap.

## crates.io

`groovyfmt` 0.9.2 was published to crates.io by hand on 2026-11-12, from the `v0.9.2` tag, with a short-lived API token (crates.io allows trusted publishing only for a crate that already exists). Later versions publish from `release.yml`'s `publish-crate` job:

- crates.io trusts this repository's `release.yml` (crate settings, Trusted Publishing: owner `costajohnt`, repository `groovyfmt`, workflow `release.yml`, no environment).
- `rust-lang/crates-io-auth-action` trades the job's OIDC token for a publish token, and revokes it when the job ends. No token secret is stored.
- The job runs `cargo publish --locked` from the tag, after `release` succeeds, when `PUBLISH_CRATE` is `true`.

A version can't be replaced once published, only yanked. Once a CI publish has worked, the crate's "Require trusted publishing for all new versions" setting can be switched on so an API token can no longer publish it.

## Going public, in order

1. Merge the install-channel changes and cut a release, so a tag carries `action.yml` and `.pre-commit-hooks.yaml`.
2. Make the repository public. Release downloads, the action, the pre-commit hooks and `cargo install --git` then work without credentials.
3. npm: claim the scope and set up trusted publishing as above, then set `PUBLISH_NPM`.
4. Docker: set `PUBLISH_IMAGE`.
5. Gradle plugin: create the portal account and API key secrets as above, then set `PUBLISH_GRADLE_PLUGIN`.
   VS Code extension: create the Marketplace publisher and trusted publishing policy and the Open VSX namespace and `OVSX_PAT` as above, then set `PUBLISH_VSCODE` and `PUBLISH_OPENVSX`.
   JetBrains plugin: create the vendor, upload the first version by hand and store the token (and the signing secrets, if signing) as above, then set `PUBLISH_JETBRAINS`.
6. Push the next release tag. Check that `publish-npm`, `publish-image` and `publish-gradle-plugin` succeeded, `npx groovyfmt --version` prints the new version, and `docker run --rm ghcr.io/costajohnt/groovyfmt --version` too (after making the package public).
7. Homebrew: create the tap from that release as above.
8. In `README.md` (Install) and `docs/ci.md` (Gradle), drop the "not published yet" lines for each channel that is now published.
