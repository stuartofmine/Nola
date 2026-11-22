#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
set -euo pipefail
. "$(dirname "$0")/lib/install_mac_executable.sh"
fixture=$(mktemp -d "${TMPDIR:-/tmp}/nola-runtime-install-test.XXXXXX")
destination="$fixture/helper"
cleanup() {
    rm -f "$destination"
    rm -rf "$fixture/products" "$fixture/archive" "$fixture/uninstalled"
    rmdir "$fixture"
}
trap cleanup EXIT
cp /usr/bin/true "$destination"
original_inode=$(stat -f %i "$destination")
# Failure must preserve the existing executable even when called in an if.
if nola_install_mac_executable "$fixture/does-not-exist" "$destination" NO; then
    echo 'Missing source unexpectedly succeeded' >&2
    exit 1
fi
[[ $(stat -f %i "$destination") == "$original_inode" ]]
cmp -s /usr/bin/true "$destination"
[[ ! -e "${destination}.nola-staging" ]]
# Invalid signatures must not be installed.
if nola_install_mac_executable "$0" "$destination" NO; then
    echo 'Unsigned script unexpectedly accepted as Mach-O' >&2
    exit 1
fi
[[ $(stat -f %i "$destination") == "$original_inode" ]]
cmp -s /usr/bin/true "$destination"
[[ ! -e "${destination}.nola-staging" ]]
nola_install_mac_executable /usr/bin/true "$destination" NO
[[ $(stat -f %i "$destination") != "$original_inode" ]]
"$destination"
[[ ! -e "${destination}.nola-staging" ]]
nola_install_mac_executable /usr/bin/true "$destination" YES
"$destination"
CONFIGURATION=Release nola_install_mac_executable /usr/bin/true "$destination" YES
/usr/bin/codesign -dv --verbose=4 "$destination" 2>&1 | grep 'flags=.*runtime'
"$destination"
[[ ! -e "${destination}.nola-staging" ]]
products="$fixture/products"
metal="$fixture/uninstalled/mlx-swift_Cmlx.bundle/Contents/Resources"
mkdir -p "$products/NolaRuntime" "$metal"
printf 'fixture' > "$metal/default.metallib"
ln -s ../uninstalled/mlx-swift_Cmlx.bundle "$products/mlx-swift_Cmlx.bundle"
cp /usr/bin/true "$products/nola-mlx-runtime"
for binary in nola-agent-host nola-service; do
    cp /usr/bin/true "$products/NolaRuntime/$binary"
done
touch "$products/NolaRuntime/com.gaojc.Nola.background-scheduler.plist"
root="$(cd "$(dirname "$0")/.." && pwd)"
SRCROOT="$root" BUILT_PRODUCTS_DIR="$products" TARGET_BUILD_DIR="$fixture/archive" \
    CONTENTS_FOLDER_PATH=Nola.app/Contents UNLOCALIZED_RESOURCES_FOLDER_PATH=Nola.app/Contents/Resources \
    CODE_SIGNING_ALLOWED=NO bash "$root/scripts/embed_mac_runtime.sh"
bundle="$fixture/archive/Nola.app/Contents/Resources/mlx-swift_Cmlx.bundle"
[[ ! -L "$bundle" ]]
cmp "$bundle/Contents/Resources/default.metallib" "$metal/default.metallib"
echo 'PASS: failed copies/signatures preserve output; replacement changes inode; staging cleaned; both signing modes execute.'
