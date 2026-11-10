#!/bin/sh
set -eu
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
# The production projection now consumes Core/Store receipts rather than owning
# a second lifecycle reducer. Test the real app module and observation entry,
# in Products/Test; never compile a mock standalone reducer or replace Debug.
exec "$ROOT/scripts/test_mac_layers.sh" task-projection
