#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -P -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
production_bundle="$repo_root/target/dist/Prism.app"
review_bundle="$repo_root/target/dist/Prism Toolbar Review.app"
review_bundle_id="com.bentsignal.prism.toolbar-review"
review_display_name="Prism Toolbar Review"
review_base_head="c926fd4a3c7bb6b034e9278cc1e99edbf12bc3fa"

bash "$repo_root/scripts/package-prism-macos.sh" "$@"

[[ -d "$production_bundle" && ! -L "$production_bundle" ]] || {
  echo "Prism production package was not created safely: $production_bundle" >&2
  exit 1
}
if [[ -e "$review_bundle" || -L "$review_bundle" ]]; then
  [[ -d "$review_bundle" && ! -L "$review_bundle" ]] || {
    echo "refusing to replace unsafe review bundle path: $review_bundle" >&2
    exit 1
  }
  chmod -R u+w "$review_bundle"
  rm -rf -- "$review_bundle"
fi
mv "$production_bundle" "$review_bundle"

plist="$review_bundle/Contents/Info.plist"
source_commit="$(git -C "$repo_root" rev-parse HEAD)"
plutil -replace CFBundleDisplayName -string "$review_display_name" "$plist"
plutil -replace CFBundleName -string "$review_display_name" "$plist"
plutil -replace CFBundleIdentifier -string "$review_bundle_id" "$plist"
plutil -insert SpectrumReviewPrototype -string "ToolbarOverflowABC" "$plist"
plutil -insert SpectrumReviewSourceCommit -string "$source_commit" "$plist"
plutil -insert SpectrumReviewDependencyCommit -string "$review_base_head" "$plist"

[[ "$(plutil -extract CFBundleDisplayName raw -o - "$plist")" == "$review_display_name" ]]
[[ "$(plutil -extract CFBundleName raw -o - "$plist")" == "$review_display_name" ]]
[[ "$(plutil -extract CFBundleIdentifier raw -o - "$plist")" == "$review_bundle_id" ]]
[[ "$(plutil -extract SpectrumReviewPrototype raw -o - "$plist")" == "ToolbarOverflowABC" ]]
[[ "$(plutil -extract SpectrumReviewSourceCommit raw -o - "$plist")" == "$source_commit" ]]
[[ "$(plutil -extract SpectrumReviewDependencyCommit raw -o - "$plist")" == "$review_base_head" ]]

codesign --force --deep --sign - "$review_bundle"
codesign --verify --deep --strict "$review_bundle"
echo "Created $review_bundle from $source_commit (review base $review_base_head)"
