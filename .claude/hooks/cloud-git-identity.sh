#!/bin/bash
# Cloud sessions only: commit as the repository's identity (see CLAUDE.md,
# "Before every commit"). Local clones keep their own git config.
[ "$CLAUDE_CODE_REMOTE" = "true" ] || exit 0
cd "${CLAUDE_PROJECT_DIR:-.}" || exit 0
git config user.name "DEFY-AN94"
git config user.email "299535211+DEFY-AN94@users.noreply.github.com"
echo "Commit identity for this cloud session: $(git config user.name) <$(git config user.email)>"
