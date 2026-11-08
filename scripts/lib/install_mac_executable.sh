#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Replace executable inodes; overwriting a running/cached Mach-O can leave
# the kernel's code-signature cache inconsistent even when codesign verifies.
nola_install_mac_executable() (
    set -euo pipefail
    local source=$1 destination=$2 sign=${3:-NO}
    # Xcode serializes the owning build phase and grants exact output paths,
    # not arbitrary children of output directories. Declare this sibling in
    # outputPaths; it stays on the same filesystem for atomic rename.
    local staged="${destination}.nola-staging"
    trap 'rm -f "$staged"' EXIT
    cp "$source" "$staged" || exit 1
    chmod 755 "$staged" || exit 1
    cmp -s "$source" "$staged" || exit 1
    if [[ "$sign" == YES ]]; then
        /usr/bin/codesign --force --sign - --timestamp=none "$staged" || exit 1
    fi
    /usr/bin/codesign --verify "$staged" || exit 1
    mv -f "$staged" "$destination" || exit 1
)
