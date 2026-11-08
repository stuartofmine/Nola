#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Compile the production request adapter directly. No model download, GPU,
# running MLX service, application database, credentials, or network required.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
test_directory="$(mktemp -d "${TMPDIR:-/tmp}/nola-mlx-generation.XXXXXX")"
cleanup() {
    rm -r "$test_directory"
}
trap cleanup EXIT

swiftc \
    "$ROOT/NolaMLXRuntime/MLXGenerationRequest.swift" \
    "$ROOT/NolaMLXRuntimeTests/GenerationRequestTests.swift" \
    -o "$test_directory/generation-request-tests"
"$test_directory/generation-request-tests"
