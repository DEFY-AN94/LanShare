# Releasing LanShare

Pushing a tag `vX.Y.Z` makes GitHub Actions build both installers, check them and publish a GitHub Release with the installers and their checksums ([`.github/workflows/release.yml`](../.github/workflows/release.yml)). Nothing is built or uploaded from a developer machine.

LanShare never checks for updates itself. Settings › **查看新版本** (Check for new version) only opens the Releases page in the browser.

## Cut a release

1. Open a PR that:
   - sets the new version in `src-tauri/Cargo.toml`, the only file that holds it (`tauri.conf.json` and `package.json` have no version, so Tauri and the app take it from Cargo.toml), and updates `src-tauri/Cargo.lock` with `cargo check --manifest-path src-tauri/Cargo.toml`;
   - adds the release notes, in English, as `docs/releases/vX.Y.Z.md`. Start from the previous file. Leave out the downloads table and the checksums: the workflow adds both.
2. Merge the PR, then check and tag the merge commit:

   ```bash
   git fetch origin
   bash scripts/release_check.sh v0.2.0 origin/main
   git tag v0.2.0 origin/main
   git push origin v0.2.0
   ```

3. Follow the **Release** workflow in the Actions tab. It takes about 25 minutes, most of it building the Windows installer, and the release appears only after every job has passed.

## What the workflow does

| Job | What it does |
|---|---|
| Check the tag | [`scripts/release_check.sh`](../scripts/release_check.sh): the tag is `v` + the version in Cargo.toml (and Cargo.lock), optionally with a suffix; a tag without a suffix is on `main` and has its notes file |
| CI | The jobs in [`ci.yml`](../.github/workflows/ci.yml): tests on Windows and macOS, the NSIS installer and the Windows install check. The published installer is this run's artifact, the exact file the install check installed |
| macOS app | `tauri build --bundles app`, zipped with `ditto`. `signingIdentity: "-"` in `tauri.conf.json` ad-hoc signs the whole bundle; with only the executable signed, macOS calls a downloaded copy damaged. A fresh copy from the zip is checked: the signature is valid and covers the bundle, the version matches, it is arm64 for macOS 11.0, and it starts and answers on port 39210 |
| Publish the release | Writes `SHA256SUMS.txt` and the release notes (the notes file, then the downloads table and the checksums) and runs `gh release create`. The only job that can write to the repository |

A release run uses about 7 macOS runner minutes (tests 2, app 4–5) and about 25 Windows and Linux minutes (measured on the first run, 2026-10-20).

## Pre-releases and release candidates

- Versions below 1.0.0 are published as pre-releases, because the Windows version has not been tested by hand and neither build is signed with a developer certificate.
- A tag with a suffix, such as `v0.2.0-rc.1`, builds the version in Cargo.toml (the app reports 0.2.0) and is published as a pre-release with a line at the top saying which commit it was built from. It may point to any branch and needs no notes file, so it can also test a change to the release workflow before it is merged. Delete it when it is no longer needed:

  ```bash
  gh release delete v0.2.0-rc.1 --cleanup-tag --yes
  git tag -d v0.2.0-rc.1
  ```

## When something fails

Nothing is published unless every job passes.

- A job that failed for a transient reason (a runner or network problem): **Re-run failed jobs** on the workflow run.
- A real problem: fix it on `main`, delete the tag (`git push --delete origin v0.2.0 && git tag -d v0.2.0`) and tag the fixed commit again. Do this only while the release is unpublished. The repository's ruleset for `v*` tags blocks deleting or moving them, so an admin has to turn that ruleset off for this and back on right after.
- A published release that turns out to be broken: mark it as a pre-release or delete it, and publish a new patch version. Do not reuse a version number that may already have been downloaded.
