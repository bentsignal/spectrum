#!/bin/sh
set -eu

fixture_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
fixture_tmp=$(mktemp -d "${TMPDIR:-/tmp}/spectrum-font-corpus.XXXXXX")
trap 'rm -rf "$fixture_tmp"' EXIT HUP INT TERM

curl --fail --location --proto '=https' --tlsv1.2 \
  --output "$fixture_tmp/NotoSans-Regular.ttf" \
  'https://raw.githubusercontent.com/notofonts/noto-fonts/ffebf8c1ee449e544955a7e813c54f9b73848eac/hinted/ttf/NotoSans/NotoSans-Regular.ttf'

cargo run --locked -p spectrum-fonts --example generate_fixtures -- \
  "$fixture_tmp/NotoSans-Regular.ttf" "$fixture_dir"

download_fixture() {
  url=$1
  expected=$2
  filename=$3
  curl --fail --location --proto '=https' --tlsv1.2 \
    --output "$fixture_tmp/$filename" "$url"
  actual=$(shasum -a 256 "$fixture_tmp/$filename" | awk '{print $1}')
  if [ "$actual" != "$expected" ]; then
    echo "$filename SHA-256 mismatch: expected $expected, got $actual" >&2
    exit 1
  fi
  install -m 0644 "$fixture_tmp/$filename" "$fixture_dir/$filename"
}

download_fixture \
  'https://raw.githubusercontent.com/google/fonts/2f6daa88e1e71320a6fe71cc91ecbfc018928737/ofl/notosans/NotoSans%5Bwdth%2Cwght%5D.ttf' \
  'bfb7bb691513f12e734dc346c03a03f784912432d7e3fa8e56efcf906fe86b3d' \
  'noto-sans-variable-rejected.ttf'
download_fixture \
  'https://raw.githubusercontent.com/google/fonts/2f6daa88e1e71320a6fe71cc91ecbfc018928737/ofl/notosans/OFL.txt' \
  'cee9892f9f0cc8fe882c9e9537ee6a89621d86ee7ceaf70b02e2b2b1c25c061a' \
  'OFL-google-fonts.txt'
download_fixture \
  'https://raw.githubusercontent.com/notofonts/noto-fonts/ffebf8c1ee449e544955a7e813c54f9b73848eac/unhinted/otf/NotoSans/NotoSans-Regular.otf' \
  '7b8a545d63de82a3325dc3c545b597898c03219bd432b0a18086e7605859c6c4' \
  'noto-sans-cff-rejected.otf'
download_fixture \
  'https://raw.githubusercontent.com/notofonts/noto-fonts/ffebf8c1ee449e544955a7e813c54f9b73848eac/LICENSE' \
  '0dab92d0544f7b233403f14b84a663bdbfa746982eda629e7f4f9ffe1b036feb' \
  'LICENSE-notofonts.txt'
