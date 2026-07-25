#!/usr/bin/env bash
set -euo pipefail

if [[ "$#" -ne 1 ]]; then
  echo "usage: $0 <Info.plist>" >&2
  exit 2
fi

repo_root="$(cd -P -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
plist="$1"
[[ -f "$plist" && ! -L "$plist" ]] || {
  echo "bundle Info.plist must be a regular file: $plist" >&2
  exit 1
}

git_revision="$(git -C "$repo_root" rev-parse --verify HEAD^{commit})"
build_number="$(git -C "$repo_root" rev-list --count HEAD)"
git_dirty=false
if [[ -n "$(git -C "$repo_root" status --porcelain --untracked-files=normal)" ]]; then
  git_dirty=true
fi

set_string() {
  local key="$1"
  local value="$2"
  if plutil -extract "$key" raw -o - "$plist" >/dev/null 2>&1; then
    plutil -replace "$key" -string "$value" "$plist"
  else
    plutil -insert "$key" -string "$value" "$plist"
  fi
}

set_boolean() {
  local key="$1"
  local value="$2"
  if plutil -extract "$key" raw -o - "$plist" >/dev/null 2>&1; then
    plutil -replace "$key" -bool "$value" "$plist"
  else
    plutil -insert "$key" -bool "$value" "$plist"
  fi
}

set_string CFBundleVersion "$build_number"
set_string SpectrumGitRevision "$git_revision"
set_boolean SpectrumGitDirty "$git_dirty"

[[ "$(plutil -extract CFBundleVersion raw -o - "$plist")" == "$build_number" ]]
[[ "$(plutil -extract SpectrumGitRevision raw -o - "$plist")" == "$git_revision" ]]
[[ "$(plutil -extract SpectrumGitDirty raw -o - "$plist")" == "$git_dirty" ]]
