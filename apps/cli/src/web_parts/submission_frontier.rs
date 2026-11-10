// SPDX-License-Identifier: Apache-2.0
// Transport only: Core validates source ownership and observes the frontier.
fn submission_frontier_response(request: &mut Request) -> Response<Box<dyn Read + Send>> {
    let Some(token) = mobile_bearer_token(request) else {
        return json_response(StatusCode(401), json!({"error":"unauthorized"}));
    };
    let body = match read_mobile_json_value(request, &token) {
        Ok(body) => body,
        Err(response) => return response,
    };
    let input: nola_domain::remote_submission::SubmissionFrontierRequest = match serde_json::from_value(body) {
        Ok(input) => input,
        Err(_) => return mobile_encrypted_json_response(StatusCode(422),
            json!({"code":"invalid_submission_frontier_request"}), &token),
    };
    let result = crate::application_port::CliApplicationPort::persistent_default()
        .and_then(|mut application| application.execute::<nola_domain::remote_submission::SubmissionFrontier>(
            nola_application::ApplicationCommand::ObserveSubmissionFrontier {
                source_host_id: host_identity().into(), request: input,
            }));
    let (status, body) = match result {
        Ok(frontier) => (StatusCode(200), json!({"source_host_id":host_identity(),"result":frontier})),
        Err(error) => (StatusCode(409), json!({"code":"admission_frontier_unavailable","error":error})),
    };
    mobile_encrypted_json_response(status, body, &token)
}
