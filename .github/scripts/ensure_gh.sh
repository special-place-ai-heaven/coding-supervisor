#!/bin/bash
# Copied from symforge execution/ensure_gh.sh: install gh on a runner that lacks it.
set -euo pipefail
dest="${HOME}/.local/bin"
if command -v gh >/dev/null 2>&1; then
  exit 0
fi
if [ -x "${dest}/gh" ]; then
  if [ -n "${GITHUB_PATH:-}" ]; then
    echo "$dest" >> "${GITHUB_PATH}"
  fi
  exit 0
fi
mkdir -p "$dest"
tmp="$(mktemp -d)"
ver="2.102.0"
curl -fsSL -o "$tmp/gh.tgz" "https://github.com/cli/cli/releases/download/v${ver}/gh_${ver}_linux_amd64.tar.gz"
tar -xz -C "$tmp" -f "$tmp/gh.tgz"
bin="$(find "$tmp" -type f -name gh -print -quit)"
install -m 755 "$bin" "${dest}/gh"
if [ -n "${GITHUB_PATH:-}" ]; then
  echo "$dest" >> "${GITHUB_PATH}"
fi
