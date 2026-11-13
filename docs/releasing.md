# Releasing and publishing

## A release

1. Bump `version` in `Cargo.toml` (and `Cargo.lock`), add a `## <version> (<date>)` heading to `CHANGELOG.md`, and update the version pinned in the examples (`costajohnt/groovyfmt@v<version>`, `rev: v<version>`, `version: <version>`, `version = '<version>'`) in `README.md` and `docs/ci.md`; `grep -rnF '<old version>' README.md docs/ci.md` finds them. Merge to `main`.
2. Run `gh workflow run ci.yml --ref main` and wait for it to pass. Pull requests and pushes skip the two macOS release builds and the macOS action and npm jobs (hosted macOS runners are scarce); this run has them, so a macOS break shows up before the tag does.
3. Tag the merge commit `v<version>` and push the tag.

`release.yml` then checks the tag (`check`), runs all of `ci.yml` on it (`ci`), and creates the GitHub release with the five archives and their `.sha256` files (`release`): a `.tar.gz` for each Linux and macOS target and a `.zip` for Windows. The pre-commit hooks (`.pre-commit-hooks.yaml`) and the GitHub Action (`action.yml`) need nothing more: they are used straight from the tag.

Three more jobs publish to registries. Each is off until a repository variable switches it on, and each runs only after `release` succeeds, on the tag push itself. Rerunning a whole release workflow fails at `release` because the GitHub release already exists, so switch a channel on before pushing the tag you want it to publish.

| Variable | Job | Publishes |
|---|---|---|
| `PUBLISH_NPM` = `true` | `publish-npm` | `groovyfmt`, `@groovyfmt/{darwin,linux}-{arm64,x64}` and `@groovyfmt/win32-x64` to npm, with provenance |
| `PUBLISH_IMAGE` = `true` | `publish-image` | `ghcr.io/costajohnt/groovyfmt:<version>` and `:latest` for linux/amd64 and linux/arm64 |
| `PUBLISH_GRADLE_PLUGIN` = `true` | `publish-gradle-plugin` | the Gradle plugin `io.github.costajohnt.groovyfmt` to the Gradle Plugin Portal |

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

Nothing publishes to crates.io yet, and no workflow job does. The crate is ready to package: the grammar is vendored in `grammar/` (no git dependencies), `include` in `Cargo.toml` limits the package to the sources, the grammar, `build.rs`, the tests, `README.md`, `LICENSE` and `CHANGELOG.md`, and `cargo publish --dry-run --locked` succeeds. The package is MIT: the vendored grammar is `Apache-2.0 OR MIT` and groovyfmt uses it under MIT, with both of its license files kept in `grammar/`. What remains is a decision to publish and, once:

1. A crates.io account (sign in with GitHub as `costajohnt`) and an API token with the `publish-new` and `publish-update` scopes, or trusted publishing from `release.yml` once the crate exists.
2. `cargo publish --locked` from the tagged commit. A version can't be replaced once published, only yanked.

## Going public, in order

1. Merge the install-channel changes and cut a release, so a tag carries `action.yml` and `.pre-commit-hooks.yaml`.
2. Make the repository public. Release downloads, the action, the pre-commit hooks and `cargo install --git` then work without credentials.
3. npm: claim the scope and set up trusted publishing as above, then set `PUBLISH_NPM`.
4. Docker: set `PUBLISH_IMAGE`.
5. Gradle plugin: create the portal account and API key secrets as above, then set `PUBLISH_GRADLE_PLUGIN`.
6. Push the next release tag. Check that `publish-npm`, `publish-image` and `publish-gradle-plugin` succeeded, `npx groovyfmt --version` prints the new version, and `docker run --rm ghcr.io/costajohnt/groovyfmt --version` too (after making the package public).
7. Homebrew: create the tap from that release as above.
8. In `README.md` (Install) and `docs/ci.md` (Gradle), drop the "not published yet" lines for each channel that is now published.
