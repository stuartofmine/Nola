#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
set -euo pipefail
. "$(dirname "$0")/lib/install_mac_executable.sh"
fixture=$(mktemp -d "${TMPDIR:-/tmp}/nola-runtime-install-test.XXXXXX")
destination="$fixture/helper"
cleanup() {
    rm -f "$destination"
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
[[ ! -e "${destination}.nola-staging" ]]
echo 'PASS: failed copies/signatures preserve output; replacement changes inode; staging cleaned; both signing modes execute.'
