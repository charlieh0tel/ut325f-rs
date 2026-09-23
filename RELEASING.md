# Releasing

Releases are cut by pushing a `v*` tag to `main`. CI does the rest:

- `.github/workflows/build.yml` builds release binaries (x86_64 and aarch64
  Linux, x86_64 Windows) with the `btleplug` feature, and creates a GitHub
  release with those binaries attached and generated release notes.
- Once the build succeeds, the `publish-crate` job publishes the crate to
  crates.io with Trusted Publishing. No registry token is stored in the
  repository; crates.io trusts this repository and `build.yml` by name, so
  renaming that file breaks publishing until the crates.io configuration is
  updated to match.

## Steps

1. Start from an up-to-date, clean `main`:

   ```sh
   git switch main
   git pull --ff-only
   git status
   ```

2. Pick the version following [SemVer](https://semver.org/): a patch bump for
   fixes and dependency refreshes, minor for new API, major for breaking
   changes.

3. Bump `version` in `Cargo.toml`, then refresh the lockfile. Use
   `cargo update` for a full dependency refresh, or just `cargo build` to
   record the new version alone.

4. Check the tree:

   ```sh
   cargo fmt --check
   cargo clippy --all-targets --all-features
   cargo test --all-features
   cargo audit
   cargo publish --dry-run
   ```

   `cargo publish --dry-run` builds the packaged crate with default features
   only, which is what the CI publish does.

5. Commit the bump, tag it and push both:

   ```sh
   git commit -am "vX.Y.Z"
   git tag vX.Y.Z
   git push origin main vX.Y.Z
   ```

6. Watch the `Build and Release` run on GitHub Actions. When it finishes,
   check that the GitHub release has its binaries and that the new version is
   on [crates.io](https://crates.io/crates/ut325f-rs).

## If something fails

- **Build fails:** nothing has been published. Fix on `main`, then move the
  tag (`git tag -f vX.Y.Z` and `git push -f origin vX.Y.Z`), or delete it and
  release the next patch version.
- **Publish fails after the GitHub release exists:** re-run the failed
  `publish-crate` job from the Actions page. The Trusted Publishing token is
  minted per run, so a re-run gets a fresh one.
- **A crates.io version is bad:** versions cannot be overwritten or deleted.
  `cargo yank --version X.Y.Z` stops new dependents from resolving to it;
  then release a fixed patch version.
