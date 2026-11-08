// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements nola core logic in the Rust core module.
// - Interfaces:
//   - Internal implementation details only.
// - APIs:
//   - none
//
#ifndef NOLA_CORE_H
#define NOLA_CORE_H

#include <stdbool.h>
#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

char* nola_core_generate_dedupe_key(const char* source, const char* title, const char* date_iso, const char* source_id);
char* nola_core_parse_url_json(const unsigned char* data, size_t data_len);
char* nola_core_process_raw_events(const unsigned char* data, size_t data_len);
void nola_core_free_c_string(char* ptr);

// Bounded in-process Kernel presenter. Does not open a database or service.
// JSON results are owned and freed with nola_core_free_c_string.
// Keep the handle alive across batches; freeing must not race any operation.
// new accepts the Application ConversationProjectionWindowV1 response directly.
// Its stream_owners must be available (not null); event_sequence is the cursor.
// Every non-final Text Part requires Application-supplied ownership.
// apply accepts an array of ConversationProjectionEvent (maximum 256).
// Inputs and retained projection are limited to 2 MiB of encoded JSON.
typedef struct NolaCoreProjection NolaCoreProjection;
NolaCoreProjection* nola_core_projection_new(const unsigned char* json, size_t len);
char* nola_core_projection_apply(NolaCoreProjection* projection, const unsigned char* json, size_t len);
char* nola_core_projection_snapshot(NolaCoreProjection* projection);
void nola_core_projection_free(NolaCoreProjection* projection);

// Embedded Application runtime. Commands and results use the versioned JSON
// contract owned by nola-application; persistence remains an internal detail.
typedef struct NolaCoreApplication NolaCoreApplication;
uint32_t nola_core_application_abi_version(void);
uint32_t nola_core_application_service_protocol_version(void);
NolaCoreApplication* nola_core_application_open(const unsigned char* path_utf8, size_t path_len);
const char* nola_core_application_last_open_error(void);
void nola_core_application_free(NolaCoreApplication* application);
/* Trusted embedded Host factory only; never a renderer command transport.
 * payload is the current {core_config,submission} JSON. The same submission
 * must be passed to canonical admission after the witness is attached. */
char* nola_core_application_attest_resend_host_policy(
    NolaCoreApplication* application,
    const uint8_t* target_ptr, size_t target_len,
    const uint8_t* payload_ptr, size_t payload_len
);

// Trusted native Host only. Never proxy renderer Application JSON here.
char *nola_core_application_expert_host_operation(NolaCoreApplication *application, const uint8_t *input_ptr, size_t input_len);


// Query-only adapter. Mutations require versioned submission.
NolaCoreApplication* nola_core_mobile_mcp_retain(NolaCoreApplication* application);
char* nola_core_mobile_mcp_host(NolaCoreApplication* application, const unsigned char* command, size_t command_len);
char* nola_core_application_execute(
    NolaCoreApplication* application,
    const unsigned char* command_json,
    size_t command_len
);

// Durable versioned command admission. Input: CommandEnvelope; success result:
// CommandReceipt. Queries are fresh; writes are durably replayable by identity.
// command_receipt looks up a prior request in the same client scope. Free the
// returned JSON with nola_core_free_c_string, including error responses.
char* nola_core_application_submit(
    NolaCoreApplication* application,
    const unsigned char* envelope_json,
    size_t envelope_len
);

// Independent embedded child transport. Start requires an already admitted exact
// Task/Run and current authority. A polled request is not permission to execute:
// ExecutionStarted must be submitted and committed=true received first.
// close unblocks response waiters but does NOT prove physical execution stopped.
// poll waits at most 30 seconds; an empty poll is not a retry authorization.
// respond blocks for the Application commit ACK; never call it on the UI thread.
// close may run concurrently with poll/respond to release transport waiters.
// After close, wait for all native calls to return before free; free must not race.
// Free every returned JSON (including *error_json) with nola_core_free_c_string.
// A successful start retains the Application internally. The caller must keep
// its Application handle alive until start returns. A finished transport does
// not mean the child task succeeded; read its canonical terminal result.
// All JSON uses the Application frame limit.
typedef struct NolaCoreChildPump NolaCoreChildPump;
NolaCoreChildPump* nola_core_child_pump_start(NolaCoreApplication* application,
    const unsigned char* input_json, size_t input_len, char** error_json);
char* nola_core_child_pump_poll(NolaCoreChildPump* pump, uint32_t wait_ms);
char* nola_core_child_pump_respond(NolaCoreChildPump* pump, const unsigned char* response_json, size_t response_len);
void nola_core_child_pump_close(NolaCoreChildPump* pump);
void nola_core_child_pump_free(NolaCoreChildPump* pump);

// Stateless memory policy selection (ABI/schema v1). Hosts provide already
// ranked candidates and retain ownership of persistence and retrieval.
uint32_t nola_core_memory_selection_abi_version(void);
char* nola_core_memory_select_json(const unsigned char* input_json, size_t input_len);

char* nola_core_context_plan(const unsigned char* input_json, size_t input_len);
uint32_t nola_core_context_engine_abi_version(void);
char* nola_core_context_prepare(const unsigned char* input_json, size_t input_len);
// Data-source access, validation-contract sealing, and freshness resolution
// (ABI v3/schema v2). Hosts retain all source I/O and secret material.
uint32_t nola_core_data_source_abi_version(void);
char* nola_core_data_source_resolve(const unsigned char* input_json, size_t input_len);
char* nola_core_data_source_seal_contract(const unsigned char* input_json, size_t input_len);
// Trusted observation-only lifecycle Hook registry, planner, and execution
// projection (ABI v3/schema v1). The output
// never authorizes I/O and cannot mutate or block an Agent operation.
uint32_t nola_core_lifecycle_hook_abi_version(void);
char* nola_core_lifecycle_hook_build_registry(const unsigned char* input_json, size_t input_len);
char* nola_core_lifecycle_hook_plan(const unsigned char* input_json, size_t input_len);
char* nola_core_lifecycle_hook_project_execution(const unsigned char* input_json, size_t input_len);

// Prompt-cache diagnostics (digest-only; never accepts or returns prompt text).
uint32_t nola_core_prompt_cache_abi_version(void);
char* nola_core_prompt_cache_diagnose(const unsigned char* input_json, size_t input_len);
// Typed Agent prompt composition. Core owns ordering, trust validation, reference
// wrapping, and the stable cache prefix shape (ABI/schema v1).
uint32_t nola_core_prompt_composer_abi_version(void);
char* nola_core_compose_agent_prompt_json(const unsigned char* input_json, size_t input_len);
// Managed resource visibility/editing policy (ABI/schema v1). This contract
// deliberately cannot authorize Skill, MCP, tool, or data-source execution.
uint32_t nola_core_resource_access_abi_version(void);
char* nola_core_resource_access_evaluate_json(const unsigned char* input_json, size_t input_len);
// Platform-aware tool catalog and policy resolver (ABI/schema v2).
// Runtime JSON reports handler identity, schema, route, availability, and a fresh
// capability manifest. Core owns capability, risk, permission, and egress policy.
uint32_t nola_core_tool_policy_abi_version(void);
char* nola_core_tool_policy_resolve(const unsigned char* input_json, size_t input_len);
bool nola_core_tool_policy_mcp_allowed(const unsigned char* input_json, size_t input_len);
char* nola_core_parse_agent_model_output(const unsigned char* input_json, size_t input_len);
char* nola_core_stitch_agent_model_continuation(const unsigned char* input_json, size_t input_len);

// Persistent personal intelligence engine. Model output supplies candidates;
// the core owns deterministic event, memory, recall, and outcome lifecycles.
uint32_t nola_core_intelligence_abi_version(void);
char* nola_core_intelligence_ingest(const unsigned char* input_json, size_t input_len);
char* nola_core_intelligence_recall(const unsigned char* input_json, size_t input_len);
char* nola_core_intelligence_snapshot(void);
bool nola_core_intelligence_clear(void);

// Rules engine
bool nola_core_rules_init(const unsigned char* config_json, size_t config_len);
char* nola_core_rules_filter(const unsigned char* events_json, size_t events_len);
void nola_core_rules_mark_sent(const char* dedupe);
bool nola_core_rules_is_quiet_hours(void);
void nola_core_rules_clear_history(void);
// Transform model output
char* nola_core_transform_model_output(const unsigned char* output_json, size_t output_len, const char* source_id, const char* source_type);
// Decide actions given processed events and model output
char* nola_core_decide_actions(const unsigned char* processed_json, size_t processed_len, const unsigned char* model_json, size_t model_len);
// Scheduler
char* nola_core_scheduler_schedule(const unsigned char* events_json, size_t events_len, int target_hour);
// Model manager
void* nola_core_model_manager_new(void);
void nola_core_model_manager_free(void* mgr);
char* nola_core_model_sha256(const char* path);
bool nola_core_model_manager_download(void* mgr, const char* url, const char* dest);
double nola_core_model_manager_progress(void* mgr, const char* url);
bool nola_core_model_manager_cancel(void* mgr, const char* url);
char* nola_core_model_manager_model_path(void* mgr, const char* model_id);
bool nola_core_model_manager_is_downloaded(void* mgr, const char* model_id);
bool nola_core_model_selection_save_models(const unsigned char* models_json, size_t models_len);
char* nola_core_model_selection_load_models(void);
bool nola_core_model_selection_save_last_selected(const unsigned char* json, size_t len);
bool nola_core_model_selection_add_model(const unsigned char* model_json, size_t len);
bool nola_core_model_selection_remove_model(const unsigned char* model_json, size_t len);

bool nola_core_persistence_set(const char* key, const unsigned char* value, size_t value_len);
char* nola_core_persistence_get(const char* key);
bool nola_core_persistence_remove(const char* key);

// Model service API
bool nola_core_model_service_set_local_provider(const char* model_id);
bool nola_core_model_service_set_openai_provider(const unsigned char* config_json, size_t len);

// Software updates are Host operations; invoke on an IO worker.
char *nola_core_software_update_json(const uint8_t *input, size_t length);

#ifdef __cplusplus
}
#endif

#endif // NOLA_CORE_H
