#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LAYER="${1:-contracts}"
# Real UI acceptance uses the existing provider-evidence runner, including
# exact test-count and zero-skip checks. Plain xcodebuild is not acceptance.
if [[ "$LAYER" == "real-ui" ]]; then
  exec "$ROOT/scripts/test_real_mac_product_ui_multiturn.sh"
fi
MAC_ARCH="$(uname -m)"
DESTINATION="${NOLA_MAC_TEST_DESTINATION:-platform=macOS,arch=$MAC_ARCH}"

. "$ROOT/scripts/lib/xcode_test_device_guard.sh"
nola_xctest_device_guard_begin "$ROOT" macos
nola_xctest_device_guard_install_traps "$ROOT" macos

case "$DESTINATION" in
  platform=macOS,arch=*) ;;
  *)
    printf 'NOLA_MAC_TEST_DESTINATION must be an architecture-qualified macOS destination.\n' >&2
    exit 64
    ;;
esac

# These suites launch production helper processes and deliberately exercise
# termination, SIGSTOP/SIGKILL, reconnect, and crash-recovery paths. Keep them
# out of the default architecture/contract loop so a normal refactor test run
# cannot terminate a bundled Nola executable or trigger macOS CrashReporter.
host_e2e_suites=(
  MacAssistantJournalIngestionTests
  MacCancellationCLIEndToEndTests
  MacAgentCapabilityEndToEndTests
  MacBuiltInSkillEndToEndTests
  MacCanonicalStoreEndToEndTests
  MacExternalExecutionRealEndToEndTests
  MacRealProductUIEndToEndTests
  MacRemoteHostConnectionE2ETests
  MacWebSourceE2ETests
)
real_local_model_suite="MacLocalModelRealInferenceEndToEndTests"

args=(
  test
  -project "$ROOT/Nola.xcodeproj"
  -scheme NolaMac
  -configuration Test
  -destination "$DESTINATION"
  -destination-timeout 10
  ARCHS="$MAC_ARCH"
  ONLY_ACTIVE_ARCH=YES
  CODE_SIGNING_ALLOWED=NO
)

# Test configuration uses Products/Test and the tests-host identity, never
# replacing the signed Products/Debug application. An explicit DerivedData
# override additionally isolates build.db for concurrent Xcode work.
if [[ -n "${NOLA_MAC_DERIVED_DATA:-}" ]]; then
  args+=(-derivedDataPath "$NOLA_MAC_DERIVED_DATA")
fi

case "$LAYER" in
  contracts)
    for suite in \
      "${host_e2e_suites[@]}" \
      "$real_local_model_suite"; do
      args+=("-skip-testing:NolaMacTests/$suite")
    done
    ;;
  host-e2e)
    for suite in "${host_e2e_suites[@]}"; do
      args+=("-only-testing:NolaMacTests/$suite")
    done
    ;;
  cancel-e2e)
    args+=("-only-testing:NolaMacTests/MacCancellationCLIEndToEndTests")
    ;;
  task-projection)
    args+=("-only-testing:NolaMacTests/MacRemoteTaskObservationCoordinatorTests")
    ;;
  a2ui)
    args+=("-only-testing:NolaMacTests/MacPublicPartSemanticParityTests")
    args+=("-only-testing:NolaMacTests/MacAssistantJournalTransportTests")
    args+=("-only-testing:NolaMacTests/MacCanonicalConversationRepositoryEndToEndTests/testCanonicalCorePartsRestoreStructuredPresentationWithoutPollutingAnswerText")
    args+=("-only-testing:NolaMacTests/MacCanonicalExportTests/testPagedExportReadsDeclaredDerivationSeparatelyAndFailsIfMissing")
    ;;
  full)
    ;;
  *)
    printf 'Usage: %s [contracts|host-e2e|cancel-e2e|task-projection|a2ui|real-ui|full]\n' "$0" >&2
    exit 64
    ;;
esac

# ReportCrash is a per-user launch agent, so child-process environment alone is
# not a sufficient UI boundary on every macOS release. Suppress its dialog only
# for this test invocation and restore the user's exact prior preference on all
# normal, failure, interrupt, and termination exits.
nola_mac_crash_dialog_was_configured=0
nola_mac_crash_dialog_value=""
if nola_mac_crash_dialog_value="$(defaults read com.apple.CrashReporter DialogType 2>/dev/null)"; then
  nola_mac_crash_dialog_was_configured=1
fi
defaults write com.apple.CrashReporter DialogType none >/dev/null

nola_mac_test_cleanup() {
  nola_mac_test_status=$?
  trap - EXIT INT TERM
  if [ "$nola_mac_crash_dialog_was_configured" -eq 1 ]; then
    defaults write com.apple.CrashReporter DialogType "$nola_mac_crash_dialog_value" >/dev/null
  else
    defaults delete com.apple.CrashReporter DialogType >/dev/null 2>&1 || true
  fi
  nola_xctest_device_guard_end "$ROOT" macos
  exit "$nola_mac_test_status"
}
trap 'exit 130' INT
trap 'exit 143' TERM
trap nola_mac_test_cleanup EXIT

# Deadline/performance assertions measure real elapsed time. Maintenance sleep
# can suspend XCTest for minutes and invalidate those measurements. Scope the
# assertion to xcodebuild's lifetime; do not wake displays or unlock devices.
# -i prevents idle sleep; -s also prevents system sleep while on AC power.
/usr/bin/caffeinate -is xcodebuild "${args[@]}"
