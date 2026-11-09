#!/bin/sh
set -eu

test "$(id -u)" = 1000
git --version
ssh -V

# Exercise the same local git operations used by VaultSync without requiring
# network access, a HOME directory, or global git configuration.
unset HOME
tmp_dir="$(mktemp -d /tmp/wiki-git-smoke.XXXXXX)"
trap 'rm -rf "$tmp_dir"' EXIT HUP INT TERM

bare_repo="$tmp_dir/vault.git"
worktree="$tmp_dir/vault"
git init --bare --quiet "$bare_repo"
git init --quiet "$worktree"
cd "$worktree"
git config user.name "Wiki Git Smoke"
git config user.email "wiki-git-smoke@localhost"
printf '%s\n' 'smoke content' > README.md
git add README.md
git commit --quiet -m "smoke"
git remote add origin "file://$bare_repo"
git push --quiet --set-upstream origin HEAD:main

test "$(git --git-dir="$bare_repo" rev-parse refs/heads/main)" = "$(git rev-parse HEAD)"
printf '%s\n' 'wiki git file:// smoke passed'
