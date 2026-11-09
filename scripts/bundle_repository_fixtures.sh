#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Refreshes source fixtures used by macOS UI tests and emits an Xcode depfile.
set -euo pipefail

ROOT="${NOLA_REPOSITORY_ROOT:?NOLA_REPOSITORY_ROOT is required}"
OUTPUT_ROOT="${NOLA_FIXTURE_OUTPUT_ROOT:?NOLA_FIXTURE_OUTPUT_ROOT is required}"
DEPENDENCY_FILE="${NOLA_DEPENDENCY_FILE:?NOLA_DEPENDENCY_FILE is required}"
FIXTURES="$OUTPUT_ROOT/NolaMac"
READY="$OUTPUT_ROOT/.ready"

rm -rf "$FIXTURES"
mkdir -p "$FIXTURES" "$(dirname "$DEPENDENCY_FILE")"
ditto "$ROOT/NolaMac/UI" "$FIXTURES/UI"
ditto "$ROOT/NolaMac/Services" "$FIXTURES/Services"
mkdir -p "$OUTPUT_ROOT/Shared/Presentation"
install -m 644 "$ROOT/Shared/Presentation/AppleSoftwareUpdateSection.swift" "$OUTPUT_ROOT/Shared/Presentation/AppleSoftwareUpdateSection.swift"
for catalog in AutomationCenter.xcstrings Localizable.xcstrings ModelCenter.xcstrings; do
  install -m 644 "$ROOT/NolaMac/$catalog" "$FIXTURES/$catalog"
done
touch "$READY"

escape_dependency_path() {
  printf '%s' "$1" | sed 's/[\\ #]/\\&/g'
}

{
  printf '%s:' "$(escape_dependency_path "$READY")"
  find "$ROOT/NolaMac/UI" "$ROOT/NolaMac/Services" -type f -print | sort |
    while IFS= read -r path; do
      printf ' %s' "$(escape_dependency_path "$path")"
    done
  for path in \
    "$ROOT/NolaMac/AutomationCenter.xcstrings" \
    "$ROOT/NolaMac/Localizable.xcstrings" \
    "$ROOT/NolaMac/ModelCenter.xcstrings" \
    "$ROOT/Shared/Presentation/AppleSoftwareUpdateSection.swift" \
    "$ROOT/scripts/bundle_repository_fixtures.sh"; do
    printf ' %s' "$(escape_dependency_path "$path")"
  done
  printf '\n'
} > "$DEPENDENCY_FILE"
