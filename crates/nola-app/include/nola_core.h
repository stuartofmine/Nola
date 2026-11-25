/* SPDX-License-Identifier: Apache-2.0 */
/* Canonical C-ABI header for Nola v2 Core (`crates/nola-app`).
 * Shared by Apple (`NolaCore.framework` / Swift) and Android (JNI / C++).
 */
#ifndef NOLA_CORE_H
#define NOLA_CORE_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct NolaCoreApplication NolaCoreApplication;
typedef struct NolaCoreChildPump NolaCoreChildPump;

/* Memory management */
void nola_core_free_c_string(char *ptr);
void nola_model_privacy_free(char *ptr);

/* ABI / version introspection */
uint32_t nola_core_application_abi_version(void);
uint32_t nola_core_application_service_protocol_version(void);
uint32_t nola_core_intelligence_abi_version(void);
uint32_t nola_core_context_engine_abi_version(void);
uint32_t nola_core_memory_selection_abi_version(void);
uint32_t nola_core_data_source_abi_version(void);
uint32_t nola_core_resource_access_abi_version(void);
uint32_t nola_core_tool_policy_abi_version(void);
uint32_t nola_core_prompt_composer_abi_version(void);
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
char *nola_core_application_expert_host_operation(
    NolaCoreApplication *app,
    const uint8_t *json_ptr,
    size_t json_len
);
char *nola_core_application_attest_resend_host_policy(
    NolaCoreApplication *app,
    const uint8_t *target_ptr,
    size_t target_len,
    const uint8_t *config_ptr,
    size_t config_len
);

/* Application-scoped personal intelligence (FTS5 memory backed) */
char *nola_core_application_intelligence_ingest(
    NolaCoreApplication *app,
    const uint8_t *json_ptr,
    size_t json_len
);
char *nola_core_application_intelligence_recall(
    NolaCoreApplication *app,
    const uint8_t *json_ptr,
    size_t json_len
);
char *nola_core_application_intelligence_snapshot(NolaCoreApplication *app);
bool nola_core_application_intelligence_clear(NolaCoreApplication *app);

/* Stateless policy / context / prompt / model interop engines */
char *nola_core_context_plan(const uint8_t *json_ptr, size_t json_len);
char *nola_core_context_prepare(const uint8_t *json_ptr, size_t json_len);
char *nola_core_intelligence_recall(const uint8_t *json_ptr, size_t json_len);
char *nola_core_memory_select_json(const uint8_t *json_ptr, size_t json_len);
char *nola_core_data_source_resolve(const uint8_t *json_ptr, size_t json_len);
char *nola_core_resource_access_evaluate_json(const uint8_t *json_ptr, size_t json_len);
char *nola_core_tool_policy_resolve(const uint8_t *json_ptr, size_t json_len);
bool nola_core_tool_policy_mcp_allowed(const uint8_t *json_ptr, size_t json_len);
char *nola_core_compose_agent_prompt_json(const uint8_t *json_ptr, size_t json_len);
char *nola_core_parse_agent_model_output(const uint8_t *json_ptr, size_t json_len);
char *nola_core_stitch_agent_model_continuation(const uint8_t *json_ptr, size_t json_len);

/* Embedded child pump & mobile MCP / skill host */
NolaCoreChildPump *nola_core_child_pump_start(
    NolaCoreApplication *app,
    const uint8_t *json_ptr,
    size_t json_len,
    char **error_out
);
char *nola_core_child_pump_poll(NolaCoreChildPump *pump, uint32_t wait_ms);
char *nola_core_child_pump_respond(
    NolaCoreChildPump *pump,
    const uint8_t *json_ptr,
    size_t json_len
);
void nola_core_child_pump_close(NolaCoreChildPump *pump);
void nola_core_child_pump_free(NolaCoreChildPump *pump);

NolaCoreApplication *nola_core_mobile_mcp_retain(NolaCoreApplication *app);
char *nola_core_mobile_mcp_host(
    NolaCoreApplication *app,
    const uint8_t *json_ptr,
    size_t json_len
);
char *nola_mobile_github_skill_package_json(const char *root_path, const char *request_json);

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
