#!/usr/bin/env bash
# Remakes the font files in crates/app/assets/fonts from their official packages.
# Only needed when adding an icon or changing which letters are included.
#
# Needs: npm, and Python with fontTools and brotli
#   (pip install fonttools brotli)
#
# To add an icon: find its name on lucide.dev, look up its code in
# lucide-static's font/lucide.css (for example ".icon-box:before { content: "\e061"; }"),
# add the code to ICONS below, then add it to `icon` and `ICON_RANGES` in
# crates/app/src/theme.rs. The tests there check that the two agree.
set -euo pipefail

INTER_VERSION=4.1.1
LUCIDE_VERSION=1.70.0
ICONS="e4b3 e345 e51d e167 e1c3 e509 e4b0 e1ce e2a1 e2a0 e2e5 e2ea e2eb e45a e60e e09e e35e e43f e2e6 e4ff e18e e0b2 e0f5 e257 e0ba e061 e523 e525 e0e8 e0f3 e0e9 e076 e44c e193 e0f9 e06c e247"
LETTERS="U+0020-007E,U+00A0-00FF,U+2013,U+2014,U+2018,U+2019,U+201C,U+201D,U+2022,U+2026,U+2190-2193,U+00D7"

root="$(cd "$(dirname "$0")/.." && pwd)"
out="$root/crates/app/assets/fonts"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
cd "$work"

npm pack --silent "inter-ui@$INTER_VERSION" "lucide-static@$LUCIDE_VERSION" >/dev/null
mkdir inter lucide
tar -xzf "inter-ui-$INTER_VERSION.tgz" -C inter
tar -xzf "lucide-static-$LUCIDE_VERSION.tgz" -C lucide

for weight in Regular SemiBold; do
    python3 -I -m fontTools.subset "inter/package/web/Inter-$weight.woff2" \
        --unicodes="$LETTERS" --layout-features='kern,liga,calt,tnum' \
        --flavor= --output-file="$out/Inter-$weight.ttf"
done
cp inter/package/LICENSE.txt "$out/Inter-LICENSE.txt"

codes="$(printf 'U+%s,' $ICONS)"
python3 -I -m fontTools.subset lucide/package/font/lucide.ttf \
    --unicodes="${codes%,}" --output-file="$out/lucide-subset.ttf"
cp lucide/package/LICENSE "$out/Lucide-LICENSE.txt"

ls -l "$out"
