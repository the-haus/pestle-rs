# Releasing

A release is a `v*` tag on `main`. The [`release`](.github/workflows/release.yml)
workflow takes it from there: it checks, builds and tests everything,
publishes `@pestle-rs/node` and its eight platform packages to npm, and
creates a GitHub release with the CHANGELOG section as its notes.

## Cutting one

1. Set the version everywhere (Cargo workspace, pinned internal crates,
   `Cargo.lock`, the Node package) and open a CHANGELOG section:

   ```bash
   node scripts/bump-version.mjs 0.2.0
   ```

2. Fill in `CHANGELOG.md` under `## 0.2.0` (its text becomes the release
   notes), and replace `unreleased` with the date.
3. Check that everything agrees, and commit:

   ```bash
   node scripts/check-version.mjs        # prints 0.2.0, or what disagrees
   git commit -am "chore: release 0.2.0"
   git push
   ```

4. Wait for `ci`, `node` and `suites` to pass on `main`, then tag:

   ```bash
   git tag v0.2.0
   git push origin v0.2.0
   ```

A version with a pre-release part (`0.2.0-rc.1`) is published under npm's
`next` dist-tag and marked as a pre-release on GitHub.

## Dry runs

Run **release** from the Actions tab with `dry-run` checked (the default):
it builds and tests all eight binaries, packs the tarballs and installs
them on every platform, but publishes nothing. Run it before a release that
changes the build.

## What the pipeline guards

- the tag, `Cargo.toml`, the pinned internal crates and `package.json`
  agree, and `CHANGELOG.md` has the version's section;
- fmt, clippy and the full test suite pass in release mode;
- each platform binary is built and tested on its own OS and architecture
  (Linux builds in Node's Debian bullseye and Alpine images: glibc 2.31+
  and musl);
- every platform package has its binary and the root package lists them
  all;
- the tarballs install and work on every platform with Node 18;
- platform packages are published before the root package, and versions
  already on npm are skipped, so a failed release can be re-run.

## Secrets and settings

- `NPM_TOKEN`: an npm automation token with publish rights on the
  `@pestle-rs` scope (or configure npm trusted publishing for this
  repository and leave it unset).
- The `npm-publish` environment: add required reviewers to it to gate
  every publish behind an approval.
- The repository is public, so packages are published with npm provenance.
