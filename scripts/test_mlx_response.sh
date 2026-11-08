#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
test_directory="$(mktemp -d "${TMPDIR:-/tmp}/nola-mlx-response.XXXXXX")"
cleanup() { rm -r "$test_directory"; }
trap cleanup EXIT
swiftc "$ROOT/NolaMLXRuntime/MLXChatResponse.swift" \
    "$ROOT/NolaMLXRuntimeTests/ChatResponseTests.swift" \
    -o "$test_directory/chat-response-tests"
"$test_directory/chat-response-tests"
swiftc "$ROOT/NolaMLXRuntime/MLXHTTPResponseWriter.swift" \
    "$ROOT/NolaMLXRuntimeTests/HTTPResponseWriterTests.swift" \
    -o "$test_directory/http-response-writer-tests"
"$test_directory/http-response-writer-tests"
swiftc "$ROOT/NolaMLXRuntime/MLXStreamRequest.swift" \
    "$ROOT/NolaMLXRuntimeTests/StreamRequestTests.swift" \
    -o "$test_directory/stream-request-tests"
"$test_directory/stream-request-tests"
