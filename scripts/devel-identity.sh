#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
#
# Generate the nightly (Devel) application identity.
#
# Nightly builds must install alongside a stable release rather than replacing
# it. Flatpak keys installs on app-id, so a nightly sharing the stable app-id
# overwrites it. This produces .Devel variants of the desktop entry and
# metainfo, and rewrites the Flatpak manifest in place to use them.
#
# The binary side is handled separately by the `devel` cargo feature, which
# switches config::APP_ID to the same .Devel value so settings and
# secret-service credentials stay separate between channels.
#
# Run from anywhere; operates on the repository root. Intended for CI, which
# works on a throwaway checkout -- the manifest edit is destructive.

set -euo pipefail

cd "$(dirname "$0")/.."

STABLE="io.github.sethcottle.Hangar"
DEVEL="${STABLE}.Devel"

for f in "data/${STABLE}.desktop" "data/${STABLE}.metainfo.xml" "${STABLE}.yml"; do
    [ -f "$f" ] || { echo "devel-identity: missing $f" >&2; exit 1; }
done

# Desktop entry: app id appears as the Icon key. Exec stays `hangar` because
# the binary name does not change between channels.
sed -e "s|${STABLE}|${DEVEL}|g" \
    -e 's|^Name=Hangar$|Name=Hangar (Nightly)|' \
    "data/${STABLE}.desktop" > "data/${DEVEL}.desktop"

# Metainfo: <id> and the desktop-id launchable. The URLs use the lowercase
# repo path rather than the app id, so a global replace is safe here.
sed -e "s|${STABLE}|${DEVEL}|g" \
    -e 's|<name>Hangar</name>|<name>Hangar (Nightly)</name>|' \
    "data/${STABLE}.metainfo.xml" > "data/${DEVEL}.metainfo.xml"

# Flatpak manifest: app-id, the install paths for icon/desktop/metainfo, and
# the devel cargo feature. Edited in place; the filename is left alone so the
# workflow's manifest-path stays valid.
sed -i \
    -e "s|^app-id: ${STABLE}$|app-id: ${DEVEL}|" \
    -e "s|${STABLE}\.svg|${DEVEL}.svg|g" \
    -e "s|${STABLE}\.desktop|${DEVEL}.desktop|g" \
    -e "s|${STABLE}\.metainfo\.xml|${DEVEL}.metainfo.xml|g" \
    -e 's|cargo --offline build --release --verbose|cargo --offline build --release --features devel --verbose|' \
    "${STABLE}.yml"

echo "devel-identity: application identity set to ${DEVEL}"
