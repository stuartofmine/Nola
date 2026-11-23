#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Replace executable inodes; overwriting a running/cached Mach-O can leave
# the kernel's code-signature cache inconsistent even when codesign verifies.
nola_sign_embedded_binary() (
    local binary=$1
    shift
    if [[ ${CONFIGURATION:-Debug} == Release ]]; then
        local identity=${NOLA_MAC_DISTRIBUTION_IDENTITY:--}
        local timestamp=--timestamp=none
        [[ $identity == - ]] || timestamp=--timestamp
        /usr/bin/codesign --force --options runtime --sign "$identity" "$timestamp" "$@" "$binary"
    else
        /usr/bin/codesign --force --sign - --timestamp=none "$binary"
    fi
)

nola_install_mac_executable() (
    set -euo pipefail
    local source=$1 destination=$2 sign=${3:-NO}
    # Xcode serializes the owning build phase and grants exact output paths,
    # not arbitrary children of output directories. Declare this sibling in
    # outputPaths; it stays on the same filesystem for atomic rename.
    local staged="${destination}.nola-staging"
    trap 'rm -f "$staged"' EXIT
    # Prefer APFS copy-on-write clone (new inode, zero block copy) and fall back to cp.
    cp -c "$source" "$staged" 2>/dev/null || cp "$source" "$staged" || exit 1
    chmod 755 "$staged" || exit 1
    if [[ "$sign" == YES ]]; then
        nola_sign_embedded_binary "$staged" || exit 1
    fi
    /usr/bin/codesign --verify "$staged" || exit 1
    mv -f "$staged" "$destination" || exit 1
)
