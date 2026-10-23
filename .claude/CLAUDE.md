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
   - the rendered text before a PR, comment or release is posted.
7. **Security issues:** describe a fix in neutral terms. Do not publish a working exploit, or details of a weakness that is not fixed yet; private reports go through the repository's private vulnerability reporting.

## How changes land

- `main` is protected: no direct pushes or force pushes; every change goes through a PR whose CI is green (tests on Windows and macOS, the Windows installer and its install check).
- Release tags `v*` cannot be moved or deleted. Releases follow [`docs/releasing.md`](../docs/releasing.md).
- Never push branches, tags or history from another clone or repository into this one.
- GitHub content (PRs, commits, docs) is written in English; `README.zh-CN.md` is the Chinese README.
- The app's own rule still holds: logs never contain message text, file names, paths or clipboard content.
