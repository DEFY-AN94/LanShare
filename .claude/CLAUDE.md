# LanShare: rules for working in this repository

This repository is **public**. Anything committed, pushed or written on GitHub (files, commit messages, PR and issue text, review comments, release notes, Actions logs) can be read by anyone, and GitHub keeps every pull request's commits even after a branch is deleted. Privacy and security are hard requirements: when in doubt, leave it out and ask.

## Before every commit, push, PR or release

1. **Identity.** Commit as `DEFY-AN94` with `299535211+DEFY-AN94@users.noreply.github.com`. In a new clone, check `git config user.email` before the first commit. Never use a personal email address, and never add a `Co-authored-by` line with one.
2. **No personal information** in files, commit messages, PR text or images:
   - real names, personal email addresses or phone numbers;
   - the real name of any computer or phone, or its `.local` host name: write `MyMac` / `mymac.local` and `iPhone`;
   - local user names, or absolute paths under a home folder or a mounted disk: write paths relative to the repository;
   - IP addresses, MAC addresses, Wi-Fi names or device identifiers from a real network or device: use documentation values such as `192.168.1.20`;
   - names, fingerprints or serial numbers of certificates from a real install;
   - other apps someone has installed, or anything else about a person's own setup.
3. **Screenshots and recordings:** check every image before adding it, for host and device names, IP addresses, certificate names, notifications, and personal messages or files. Redact or retake. Metadata (EXIF) must be stripped.
4. **No secrets or real data:**
   - tokens, API keys, passwords, private keys, signing certificates or `.env` files;
   - the app's data folder (database, `config.json`, logs, TLS files) or files from a real shared folder;
   - logs or crash reports from a real install.

   Test data stays synthetic: `iPhone`, `Test iPad B`, `example.com`.
5. **Local-only notes stay local.** Review notes and plans kept in the working folder are excluded through `.git/info/exclude`; never stage them, and never copy their contents into public text. Use `git add <path>`, not `git add -A`, when untracked files are around.
6. **Review what goes out:**
   - `git diff --cached` before each commit;
   - `git log -p origin/main..HEAD` before each push;
   - the rendered text before a PR, comment or release is posted;
   - each PR's text and edit history right after it is opened, and again before it is merged (rule 8).
7. **Security issues:** describe a fix in neutral terms. Do not publish a working exploit, or details of a weakness that is not fixed yet; private reports go through the repository's private vulnerability reporting.
8. **No session links.** Never put a Claude Code session link (`claude.ai/code/session_…`) or a session id in commit messages, PR or issue text, comments or release notes. `.claude/settings.json` turns the link off, but cloud PRs have still been opened with it in the first version of their description and edited a few seconds later. GitHub keeps every version in the edit history, where anyone can read it. So check each PR right after opening it and again before merging; both commands should print nothing:

   ```bash
   git log --format=%B origin/main..HEAD | grep -o 'claude\.ai/code/session_[A-Za-z0-9]*'
   gh api graphql -F n=<PR number> -f query='query($n:Int!){repository(owner:"DEFY-AN94",name:"LanShare"){pullRequest(number:$n){title body userContentEdits(first:100){nodes{diff}} comments(first:100){nodes{body userContentEdits(first:100){nodes{diff}}}} reviews(first:100){nodes{body comments(first:100){nodes{body}}}}}}}' | grep -o 'claude\.ai/code/session_[A-Za-z0-9]*'
   ```

   If a link turns up, take it out of the current text. Then the repository owner deletes each version that has it, on GitHub: **edited** › the version › **Options** › **Delete revision from history**.

## How changes land

- `main` is protected: no direct pushes or force pushes; every change goes through a PR whose CI is green (tests on Windows and macOS, the Windows installer and its install check).
- Release tags `v*` cannot be moved or deleted. Releases follow [`docs/releasing.md`](../docs/releasing.md).
- Never push branches, tags or history from another clone or repository into this one.
- GitHub content (PRs, commits, docs) is written in English; `README.zh-CN.md` is the Chinese README.
- The app's own rule still holds: logs never contain message text, file names, paths or clipboard content.

## Cloud sessions

Claude Code cloud sessions (Ubuntu) work on a branch of this repository and end with a PR; the steps that need a Mac, an iPhone or a release stay in a local session.

- **Commit identity:** `.claude/hooks/cloud-git-identity.sh` runs at session start and sets `DEFY-AN94` with the noreply address in cloud sessions only. Check `git log -1 --format='%an <%ae>'` before the first push. `.claude/settings.json` turns the session link off for commits and PR descriptions. Still write the PR description without one, and run the rule 8 check on the new PR (`gh api` works through the proxy).
- **Building:** the environment's setup script installs what Tauri needs on Linux (`libwebkit2gtk-4.1-dev libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev file`, plus `rustup component add clippy`). CI only builds on Windows and macOS, so run the CI checks (`npm ci`, `npm run build`, `npm test`, then `cargo clippy --all-targets -- -D warnings` and `cargo test` in `src-tauri`) before changing anything; they pass on Linux too.
- **More checks in the cloud:** the API smoke test (`scripts/api_smoke.sh`, see `ci.yml`) needs a private IPv4 address, which the cloud VM does not have: `apt-get install -y iproute2` and `ip addr add 10.77.0.1/32 dev lo` (gone after a restart). Windows-only code compiles and lints with `apt-get install -y mingw-w64`, `rustup target add x86_64-pc-windows-gnu` and `cargo clippy --target x86_64-pc-windows-gnu --all-targets -- -D warnings`. macOS-only code cannot be built in the cloud.
- **Pushing:** push only the session's own branch and open a PR. Tags are refused and releases are cut locally (`docs/releasing.md`). `gh pr` does not work through the cloud proxy; use the Create PR button or `gh api`.
- **Not available in the cloud:** the iOS Simulator, a real iPhone, the Mac app and Windows. A local session does the acceptance checks, the review for personal data before merging, the merge and any release.
