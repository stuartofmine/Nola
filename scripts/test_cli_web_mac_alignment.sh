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

run "Core journal publication, bounded reads, and terminal admission" \
    bash -lc "cd '$ROOT' && mbx test -p nola_application -p nola_store assistant_journal -- --nocapture"

run "CLI journal consumption and authenticated transport" \
    bash -lc "cd '$ROOT' && mbx test -p nola_cli --lib journal -- --nocapture"

run "Host real HTTP submission, receipt, and model configuration" \
    bash -lc "cd '$ROOT' && mbx test -p nola_cli --test conversation_configuration_web_e2e -- --nocapture"

run "Web production submission and journal observation contracts" \
    node --test \
        "$ROOT/apps/web/frontend/src/lib/sourceSubmission.test.ts" \
        "$ROOT/apps/web/frontend/src/lib/runObservation.test.ts" \
        "$ROOT/apps/web/frontend/src/lib/chatTurn.test.ts" \
        "$ROOT/apps/web/frontend/src/lib/chatFlow.test.ts"

run "Web current Markdown renderer regressions" \
    node --test "$ROOT/apps/web/frontend/src/lib/markdown.test.ts"

run "macOS real journal reads, terminal, and visible-run lifecycle" \
    xcodebuild test -quiet \
        -project "$ROOT/apple/Nola.xcodeproj" \
        -scheme NolaMac \
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
