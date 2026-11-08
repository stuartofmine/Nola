// SPDX-License-Identifier: Apache-2.0
// Keeps the framework's public ABI anchored to the Rust archive.

#include "nola_core.h"

uint32_t nola_core_framework_abi_version(void) {
    return nola_core_application_abi_version();
}
