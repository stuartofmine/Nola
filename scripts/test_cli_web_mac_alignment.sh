#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MAC_ARCH="$(uname -m)"
MAC_DESTINATION="${NOLA_MAC_TEST_DESTINATION:-platform=macOS,arch=$MAC_ARCH}"

case "$MAC_DESTINATION" in
    platform=macOS,arch=*) ;;
    *)
        printf 'NOLA_MAC_TEST_DESTINATION must be an architecture-qualified macOS destination.\n' >&2
        exit 64
        ;;
esac

run() {
    printf '\n==> %s\n' "$1"
    shift
    "$@"
}

run "Cross-platform Warm Paper/Ink design tokens and full-source enforcement" \
    python3 "$ROOT/scripts/check_design_tokens.py"

run "Design-token enforcement mutation regression tests" \
    python3 -m unittest discover -s "$ROOT/scripts" -p 'test_check_design_tokens.py' -v

run "Core journal publication, bounded reads, and terminal admission" \
    bash -c "bash '$ROOT/scripts/test_rust.sh' nola_application assistant_journal -- --nocapture && bash '$ROOT/scripts/test_rust.sh' nola_store assistant_journal -- --nocapture"

run "CLI journal consumption and authenticated transport" \
    bash -c "bash '$ROOT/scripts/test_rust.sh' nola_cli journal -- --nocapture"

run "Host real HTTP submission, receipt, and model configuration" \
    bash -c "bash '$ROOT/scripts/test_rust.sh' nola_cli conversation_configuration_web_e2e --test web_host_e2e -- --nocapture"

run "Web frontend typecheck, full unit suite, and production build" \
    bash -c "cd '$ROOT/apps/web/frontend' && npm run check && npm test && npm run build"

run "Web Playwright runtime unit suite and visual/workflow regressions" \
    bash -c "cd '$ROOT' && cargo build --locked --manifest-path '$ROOT/Cargo.toml' -p nola_cli --bin nola && cd '$ROOT/apps/web/playwright_runtime' && npm test && npm run test:ui"

run "macOS real journal reads, terminal, and visible-run lifecycle" \
    xcodebuild test -quiet \
        -project "$ROOT/apple/Nola.xcodeproj" \
        -scheme NolaMac \
        -configuration Test \
        -destination "$MAC_DESTINATION" \
        -destination-timeout 10 \
        ARCHS="$MAC_ARCH" \
        ONLY_ACTIVE_ARCH=YES \
        CODE_SIGNING_ALLOWED=NO \
        -only-testing:NolaMacTests/MacAssistantJournalIngestionTests \
        -only-testing:NolaMacTests/MacChatWorkspaceBehaviorTests/testJournalKeepsPartIdentityAndOldSnapshotsAcrossUnicodeAppend \
        -only-testing:NolaMacTests/MacChatWorkspaceBehaviorTests/testJournalPublishesTextBeforeTerminalAndDoesNotReplayAtCompletion \
        -only-testing:NolaMacTests/MacChatWorkspaceBehaviorTests/testSealedAnswerHasOneOwnerAndStopsVisibleRun \
        -only-testing:NolaMacTests/MacChatWorkspaceBehaviorTests/testCoreTerminalIsRecognizedWithoutWaitingForProcessEOF \
        -only-testing:NolaMacTests/MacChatWorkspaceBehaviorTests/testVisibleRunStateStopsAtTerminalBeforeInternalFinalizationCompletes \
        -only-testing:NolaMacTests/MacChatWorkspaceBehaviorTests/testComposerSurfaceIsOpaqueOverContrastingContent

printf '\nCLI / Web / macOS alignment gate passed.\n'
