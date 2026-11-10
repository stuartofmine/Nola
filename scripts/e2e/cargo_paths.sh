#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Canonical Cargo artifact resolution for repository end-to-end tests.

nola_cargo_target_directory() {
    local repository_root=$1
    local manifest_path=${2:-"$repository_root/apps/cli/Cargo.toml"}

    cargo metadata \
        --manifest-path "$manifest_path" \
        --format-version 1 \
        --no-deps \
        | python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])'
}

nola_cargo_binary() {
    local repository_root=$1
    local profile=${2:-release}
    local binary_name=${3:-nola}
    local manifest_path=${4:-"$repository_root/apps/cli/Cargo.toml"}
    local target_directory

    target_directory="$(nola_cargo_target_directory "$repository_root" "$manifest_path")"
    printf '%s/%s/%s\n' "$target_directory" "$profile" "$binary_name"
}

nola_cargo_example() {
    local repository_root=$1
    local profile=$2
    local example_name=$3
    local manifest_path=$4
    local target_directory

    target_directory="$(nola_cargo_target_directory "$repository_root" "$manifest_path")"
    printf '%s/%s/examples/%s\n' "$target_directory" "$profile" "$example_name"
}
