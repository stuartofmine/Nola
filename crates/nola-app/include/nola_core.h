/* SPDX-License-Identifier: Apache-2.0 */
/* Canonical C-ABI header for Nola v2 Core (`crates/nola-app`).
 * Shared by Apple (`NolaCore.framework` / Swift) and Android (JNI / C++).
 */
#ifndef NOLA_CORE_H
#define NOLA_CORE_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct NolaCoreApplication NolaCoreApplication;

/* Memory management */
void nola_core_free_c_string(char *ptr);
void nola_model_privacy_free(char *ptr);

/* ABI / version introspection */
uint32_t nola_core_application_abi_version(void);
uint32_t nola_core_application_service_protocol_version(void);
const char *nola_v2_version(void);

/* Application store & command execution */
NolaCoreApplication *nola_core_application_open(const uint8_t *path_utf8, size_t path_len);
char *nola_core_application_last_open_error(void);
void nola_core_application_free(NolaCoreApplication *app);
char *nola_core_application_execute(
    NolaCoreApplication *app,
    const uint8_t *json_ptr,
    size_t json_len
);
char *nola_core_application_submit(
    NolaCoreApplication *app,
    const uint8_t *json_ptr,
    size_t json_len
);

/* Cloud egress secret/credential redaction */
char *nola_core_redact_model_request_json(
    const uint8_t *input,
    size_t size,
    const uint8_t *credential,
    size_t credential_len
);

/* Software update manifest check */
char *nola_core_software_update_json(const uint8_t *input, size_t size);

/* Token estimation */
size_t nola_v2_estimate_tokens(const char *text);

#ifdef __cplusplus
}
#endif

#endif /* NOLA_CORE_H */
