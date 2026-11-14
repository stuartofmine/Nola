// SPDX-License-Identifier: Apache-2.0
//! Synthetic issuer, signing key, tokens and loopback HTTP. No OpenAI network.

// Generated only for these loopback tests; not a production credential.
const KEY: &str = r#"-----BEGIN PRIVATE KEY-----
MIIEvAIBADANBgkqhkiG9w0BAQEFAASCBKYwggSiAgEAAoIBAQCk7f1/bebWiRx4
2Oy8ufD2m7Yg+dXasDEcMCnON58nXd1TjtnKsiB4D4vscowEKNk3RnHXNEXq9+KV
XjPLa0+SsT2ugXDE5LAOqPBT4bFAQ+dPNB+4mHI1jnDr/SxhxSvi/YjoGE0j46Gf
xJFx1SZWZJaG32mNFYTYAN9qNx2LNzkGX4QqEDYmsQuwG3hYGQoGFSaYuYkUFLm1
LwjC8UCNDBuCxQ8D2IphzEWopTXV7tLHpFtmw08dAW2HkM4SQO+ChyB/n2YMrcrg
aD+cvn5B2k/L5IbYd1bNLsbzjB/preHl2mp+L+1WF9K183636hZ4x8yJgyBjYgyk
oBJA2S7PAgMBAAECggEAIkNq6YE8do9Pge+W5vI+FA1vPdSJUABQg4mh9bxqbPQ8
GbLQniJ5Slf4equlWOvfjHMvDUVHzTOoN921JuppQpFdvhYYBl94D+Oa4eI+P5qb
wEGU4XWSLHwmC3CyOrGKl4KSYZ1C0VTl69obWSmt6D7HYsehFvVEftdXkqpWkegW
iHXlbq0KISbqE/njg6PgAkO18weql8SXjE4xZUFM/O8+plSBgFF1/nZ6YvrAzNi4
CE/r08sNtoFky9uK5PXxI7fskKaCq6ozFtcQAjAXtzA9l7S5+/wVEGF4bCPbV33D
CRujwPbPWEjvlUFQgXotxycxTDY//VU6/ogpYcnOeQKBgQDZElhEolAWDf0VlsIn
AThY6EJwjvAGZkjw2h+OgDX/Ii5xUaqNL5PxTUrV6S9SEbTm2zsS7mJJdsxCSltl
bU/wYtrj4DzzIbLAV+WOOTVRvWT8oABpBhN9XXrantBiDEwJXL2WVcYENMERwIeX
6u1jkVsKy0z5GorwQLsKmWVSBQKBgQDCgdS1UaovqV7uR3Q3Hkrz+gJQ0+MDnsm1
2RdOH0GdnzVebK/JIfY42U44dYZ6WMcAThyHCFEMihOjSkn70rXg853du5/8AXih
LjKm/HUAFMVCeb9YFAxSSr7LfWYHi97C/XObMV8+YVtrWJIOkpKgo+eRcyjH5+OS
LOS/3bHxwwKBgCxY2Bmlg3EOCI95jk7DrAxV3wT53sWf7fTuyxWk0eM+y3gxNAPG
HZZlsQ1Ep3s/zNKBJW93rjLOB2rfoA53p6Y3u/pkHG+bidZL3sUQoPkDFXebBJpw
OFkbWPcDDRWK55neDYmCGoNPd8Y1rD34K+l0j8IDyMT3jD0LLGs87Xs1AoGAIFIG
kGKTjvvr5FhHPWlxNv1SKTAsRsu55NfAAaE7+/8alSgmRZu5GFUzpDoTMH1UA/72
ypFdNmIzRSzxAkFPK6QPlWq5+NwduVq92/WTU4IgDR3B59bc/UeiUJnzvElHqMsI
zL71fp4a3MmI65b7emoB7q9eFWm47O88y40BJpUCgYBM3jh1g0IXrPB3ryoxaXGr
5422Jd4gL4xae4B/i5fvu0uLyXLYRw7xmUXE2IVFj3gt/p9p/PhNkT6+A/0BFMJ7
72Q4hJ+DSfITny+AnOCSLitzcjsk6mOHSdy6krIWyrUNlokIcdeRC/antxQm5kGS
6o95vyW6ODUnzXgKO/wWZQ==
-----END PRIVATE KEY-----
"#;
const MODULUS: &str = "A4EDFD7F6DE6D6891C78D8ECBCB9F0F69BB620F9D5DAB0311C3029CE379F275DDD538ED9CAB220780F8BEC728C0428D9374671D73445EAF7E2955E33CB6B4F92B13DAE8170C4E4B00EA8F053E1B14043E74F341FB89872358E70EBFD2C61C52BE2FD88E8184D23E3A19FC49171D52656649686DF698D1584D800DF6A371D8B3739065F842A103626B10BB01B7858190A06152698B9891414B9B52F08C2F1408D0C1B82C50F03D88A61CC45A8A535D5EED2C7A45B66C34F1D016D8790CE1240EF8287207F9F660CADCAE0683F9CBE7E41DA4FCBE486D87756CD2EC6F38C1FE9ADE1E5DA6A7E2FED5617D2B5F37EB7EA1678C7CC89832063620CA4A01240D92ECF";

use super::*;
use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};

struct Fixture {
    host: SiwcHost, state: Arc<Mutex<Value>>, stop: Arc<AtomicBool>,
    server: Option<std::thread::JoinHandle<()>>, _root: tempfile::TempDir,
}
impl Drop for Fixture {
    fn drop(&mut self) { self.stop.store(true,Ordering::SeqCst); self.server.take().unwrap().join().unwrap(); }
}
impl Fixture {
    fn new() -> Self {
        let root=tempfile::tempdir().unwrap();let database=root.path().join("fixture.sqlite");File::create(&database).unwrap();
        let server=tiny_http::Server::http("127.0.0.1:0").unwrap();let origin=format!("http://{}",server.server_addr());
        let state=Arc::new(Mutex::new(json!({"nonce":"","subject":"subject-one","scope":SCOPES,"refresh_failure":false,"revoke_failure":false,"reasoning_levels":[],"summary_supported":null,"requests":[]})));
        let shared=state.clone();let stop=Arc::new(AtomicBool::new(false));let stopped=stop.clone();let issuer=origin.clone();
        let thread=std::thread::spawn(move || {
            while !stopped.load(Ordering::SeqCst) {
                let Some(mut request)=server.recv_timeout(Duration::from_millis(25)).unwrap() else {continue};
                let path=request.url().to_owned();let mut body=String::new();request.as_reader().read_to_string(&mut body).unwrap();
                let mut state=shared.lock().unwrap();state["requests"].as_array_mut().unwrap().push(json!({"path":path,"body":body}));
                let mut status=200;let mut content_type="application/json";
                let payload=match path.as_str() {
                    "/.well-known/openid-configuration"=>json!({"issuer":issuer,"authorization_endpoint":format!("{issuer}/authorize"),"token_endpoint":format!("{issuer}/token"),"jwks_uri":format!("{issuer}/jwks"),"revocation_endpoint":format!("{issuer}/revoke")}).to_string(),
                    "/jwks"=>json!({"keys":[{"kty":"RSA","kid":"fixture","use":"sig","alg":"RS256","n":URL_SAFE_NO_PAD.encode(hex::decode(MODULUS).unwrap()),"e":"AQAB"}]}).to_string(),
                    "/token"=>{
                        let fields:BTreeMap<_,_>=url::form_urlencoded::parse(body.as_bytes()).into_owned().collect();
                        assert_eq!(fields["client_id"],"oaiapp_fixture");assert_eq!(fields["resource"],format!("{issuer}/v1"));assert!(!fields.contains_key("client_secret"));
                        let refresh=fields["grant_type"]=="refresh_token";
                        if refresh && state["refresh_failure"]==true {status=503;json!({"error":"temporary_failure"}).to_string()}
                        else {
                            if refresh {assert_eq!(fields["refresh_token"],"refresh-one");assert!(!fields.contains_key("scope"));}
                            else {assert!(fields["code_verifier"].len()>=43);assert!(fields["redirect_uri"].starts_with("http://127.0.0.1:"));}
                            let now=chrono::Utc::now().timestamp();
                            let claims=json!({"iss":issuer,"aud":"oaiapp_fixture","sub":state["subject"],"email":"same@example.invalid","iat":now,"exp":now+3600,"nonce":state["nonce"]});
                            json!({"access_token":if refresh {"access-two"} else {"access-one"},"refresh_token":if refresh {"refresh-two"} else {"refresh-one"},"id_token":signed(&claims),"token_type":"Bearer","expires_in":3600,"scope":state["scope"]}).to_string()
                        }
                    },
                    "/revoke"=>{if state["revoke_failure"]==true {status=503;}String::new()},
                    "/v1/models"=>json!({"models":[{"slug":"hidden","display_name":"Hidden","visibility":"hide"},{"slug":"exact:slug","display_name":"Fixture model","visibility":"list","supported_reasoning_levels":state["reasoning_levels"],"supports_reasoning_summary_parameter":state["summary_supported"]}]}).to_string(),
                    "/v1/responses"=>{
                        let expected = state.get("expected_response_body").cloned().unwrap_or_else(|| json!({"model":"exact:slug","input":[{"role":"user","content":"fixture only"}],"stream":true,"store":false}));
                        assert_eq!(serde_json::from_str::<Value>(&body).unwrap(), expected);
                        let bearer = state["expected_bearer"].as_str().unwrap_or("Bearer access-one");
                        assert_eq!(request.headers().iter().find(|h|h.field.equiv("Authorization")).unwrap().value.as_str(), bearer);
                        content_type="text/event-stream";"data: {\"type\":\"response.completed\",\"response\":{\"id\":\"fixture-response\",\"usage\":{\"input_tokens\":4,\"output_tokens\":1}}}\n\n".into()
                    },_=>{status=404;String::new()},
                };drop(state);
                request.respond(tiny_http::Response::from_string(payload).with_status_code(status).with_header(tiny_http::Header::from_bytes("Content-Type",content_type).unwrap())).unwrap();
            }
        });
        let mut host=SiwcHost::open(&database).unwrap();host.endpoints=Endpoints {issuer:origin.clone(),resource:format!("{origin}/v1")};
        Self {host,state,stop,server:Some(thread),_root:root}
    }
    fn begin(&self,account:&str,revision:i64)->AuthorizationAttempt {
        let attempt=self.host.begin_sign_in(account,revision).unwrap();self.state.lock().unwrap()["nonce"]=json!(attempt.nonce);attempt
    }
    fn sign_in(&self,account:&str)->Snapshot {
        let attempt=self.begin(account,0);self.host.exchange_callback(&attempt,&callback(&attempt)).unwrap()
    }
    fn count(&self,path:&str)->usize {self.state.lock().unwrap()["requests"].as_array().unwrap().iter().filter(|r|r["path"]==path).count()}
}
fn callback(a:&AuthorizationAttempt)->String {format!("/auth/callback?state={}&code=fake-code&client_id=oaiapp_fixture",a.state)}

#[test]
fn model_siwc_mac_admission_binds_exact_profile_without_api_secret() {
    let _environment=crate::store::test_env_lock();
    let f=Fixture::new();
    let id=uuid::Uuid::new_v4().to_string();
    let session=f.sign_in(&id);
    let store=crate::store::NolaStore::open(&f.host.database).unwrap();
    let owner=crate::model_siwc_cli::save_account(&store,&id,"fixture",None,&session).unwrap();
    let connection=crate::model_siwc_cli::sync_catalog(&store,&owner,vec![ModelChoice {slug:"exact:slug".into(),display_name:"Fixture".into(),
        supported_reasoning_levels: ["low", "medium", "high"].into_iter()
            .map(|effort| CatalogReasoningLevel { effort: effort.into() }).collect(), ..Default::default()}]).unwrap();
    let connection: nola_domain::model_connection::ModelConnectionSnapshot = store.application().unwrap().execute(
        nola_application::ApplicationCommand::Model { command: nola_application::ModelCommand::SetConnectionCapabilityOverride {
            connection_id: connection.document.connection.id, expected_revision: connection.revision,
            model_id: "exact:slug".into(), protocol: nola_domain::generation::ModelProtocol::SiwcResponses,
            capabilities: Some(nola_application::ModelExecutionCapabilities::agent_baseline()),
            updated_at_ms: chrono::Utc::now().timestamp_millis(),
        }}).unwrap();
    let _:Value=store.application().unwrap().execute(nola_application::ApplicationCommand::Model {command:nola_application::ModelCommand::ConfigureConnectionModels {
        connection_id:connection.document.connection.id,expected_revision:connection.revision,
        enabled_model_ids:vec!["siwc_responses:exact:slug".into()],selected_model_id:Some("siwc_responses:exact:slug".into()),updated_at_ms:chrono::Utc::now().timestamp_millis(),
    }}).unwrap();
    let profile=store.model_profiles().unwrap().into_iter().find(|p|p.enabled).unwrap();
    let mut config=crate::host_invocation_request::HostInvocationRequest::for_test().unwrap();
    config.execution.execution_backend="nola".into();
    config.model_settings.endpoint="https://api.openai.com/v1".into();
    config.model_settings.model="exact:slug".into();
    config.model_settings.api_key.clear();
    config.runtime.provider=Some("siwc_responses".into());
    config.runtime.llm_fallbacks.clear();
    config.runtime.mac_launch=Some(serde_json::from_value(json!({
        "version":1,"input_origin":"interactive_user","project_path":"/workspace",
        "authorized_workspace_paths":["/workspace"],"allowed_directory_arguments":["--allow-dir","/workspace"],
        "skill_directory_arguments":["--no-skills"],"active_skill_ids":[],"model_profile_id":profile.id,"model_profile_revision":profile.revision,
        "fallback_profile_ids":[],"permission_mode":"cautious","memory_enabled":false,
        "context_window_tokens":4096,"max_iterations":0,"model_capabilities":{},"vision_input":false,
        "permission_rules":[],"host_attachments":[]
    })).unwrap());
    let request=json!({"model_profile_id":profile.id,"expected_profile_revision":profile.revision});
    crate::native_host_request::apply_siwc_profile(&mut config,&request,&store).unwrap();
    let binding=&config.runtime.siwc.as_ref().unwrap().account;
    assert_eq!(binding.account_id,id);
    assert_eq!(binding.account_revision,owner.revision);
    assert_eq!(binding.client_id,session.client_id);
    assert_eq!(binding.subject,session.subject);
    assert!(config.model_settings.api_key.is_empty());
    assert_eq!(config.runtime.mac_launch.as_ref().unwrap().permission_mode,"cautious");
    // A catalog may list effort levels while the effective profile disables reasoning.
    // Ordinary tier intent must be validated; a resend must retain its frozen
    // provider-default selection instead of inventing a new effort.
    let mut frozen = request.clone();
    frozen["reasoning_selection"] = Value::Null;
    frozen["reasoning_tier"] = json!("balanced");
    assert!(crate::native_host_request::apply_siwc_profile(&mut config,&frozen,&store).is_err());
    config.session.edited_resend_operation_id = Some(uuid::Uuid::new_v4().to_string());
    crate::native_host_request::apply_siwc_profile(&mut config,&frozen,&store).unwrap();
    assert!(config.runtime.reasoning_selection.is_none());
    assert_eq!(config.runtime.siwc.as_ref().unwrap().account.account_id, id);
    let mut missing = frozen.clone();
    missing.as_object_mut().unwrap().remove("reasoning_selection");
    assert!(crate::native_host_request::apply_siwc_profile(&mut config,&missing,&store).is_err());
    config.session.edited_resend_operation_id = None;
    let mut invalid=request.clone(); invalid["api_key"]=json!("");
    assert!(crate::native_host_request::apply_siwc_profile(&mut config,&invalid,&store).is_err());
    invalid=request.clone();invalid["expected_profile_revision"]=json!(profile.revision+1);
    assert!(crate::native_host_request::apply_siwc_profile(&mut config,&invalid,&store).is_err());
    config.runtime.mac_launch.as_mut().unwrap().model_profile_revision=Some(profile.revision+1);
    assert!(crate::native_host_request::apply_siwc_profile(&mut config,&request,&store).is_err());
    assert_eq!(f.count("/v1/responses"),0);
}
fn signed(claims:&Value)->String {
    let mut header=jsonwebtoken::Header::new(Algorithm::RS256);header.kid=Some("fixture".into());
    jsonwebtoken::encode(&header,claims,&jsonwebtoken::EncodingKey::from_rsa_pem(KEY.as_bytes()).unwrap()).unwrap()
}

fn native_service(f: &Fixture) -> crate::model_siwc_native_bridge::NativeSiwcService {
    let store=crate::store::NolaStore::open(&f.host.database).unwrap();
    let _:Value=store.application().unwrap().execute(nola_application::ApplicationCommand::Model {
        command:nola_application::ModelCommand::ListAccounts }).unwrap();
    crate::model_siwc_native_bridge::NativeSiwcService::new(store,SiwcHost {
        database:f.host.database.clone(),endpoints:f.host.endpoints.clone(),http:f.host.http.clone()
    })
}
fn native_identity(owner:&str, flow:&Value) -> crate::model_siwc_native_bridge::FlowIdentity {
    serde_json::from_value(json!({"owner_id":owner,"flow_id":flow["flow_id"],
        "account_id":flow["account_id"],"expected_account_revision":flow["expected_account_revision"]})).unwrap()
}
fn native_wait(service:&crate::model_siwc_native_bridge::NativeSiwcService, identity:&crate::model_siwc_native_bridge::FlowIdentity, phase:&str) -> Value {
    let deadline=Instant::now()+Duration::from_secs(5);
    loop {
        let flow=service.status(identity).unwrap();
        if flow["phase"]==phase {return flow;}
        assert!(Instant::now()<deadline,"native fixture did not reach phase {phase}");
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn native_authorize(f:&Fixture,service:&crate::model_siwc_native_bridge::NativeSiwcService, identity:&crate::model_siwc_native_bridge::FlowIdentity) {
    let flow=native_wait(service,identity,"awaiting_authorization");
    let url=reqwest::Url::parse(flow["authorization_url"].as_str().unwrap()).unwrap();
    let fields:BTreeMap<_,_>=url.query_pairs().into_owned().collect();
    f.state.lock().unwrap()["nonce"]=json!(fields["nonce"]);
    let mut callback=reqwest::Url::parse(&fields["redirect_uri"]).unwrap();
    callback.query_pairs_mut().append_pair("state",&fields["state"]).append_pair("code","fixture-code").append_pair("client_id","oaiapp_fixture");
    let response=reqwest::blocking::Client::builder().no_proxy().timeout(Duration::from_secs(5)).build().unwrap().get(callback).send().unwrap();
    assert!(response.status().is_success());
    native_wait(service,identity,"authorized");
}

#[test]
fn model_siwc_native_flow_is_async_identity_bound_and_cancellable() {
    let f=Fixture::new(); let mut service=native_service(&f); let owner=service.owner_id().to_string();
    let account=uuid::Uuid::new_v4().to_string(); let request_id=uuid::Uuid::new_v4().to_string();
    let request=json!({"owner_id":owner,"account_id":account,"expected_account_revision":null,"display_name":"Fixture","acknowledge_authorization":true});
    let start=Instant::now();let flow=service.start(&request_id,serde_json::from_value(request.clone()).unwrap()).unwrap();
    assert!(start.elapsed()<Duration::from_secs(1));
    let identity=native_identity(&owner,&flow);
    assert_eq!(service.start(&request_id,serde_json::from_value(request).unwrap()).unwrap()["flow_id"],flow["flow_id"]);
    assert!(service.cancel(&native_identity("different-owner",&flow)).is_err());
    let mut wrong=flow.clone();wrong["account_id"]=json!(uuid::Uuid::new_v4().to_string());
    assert!(service.cancel(&native_identity(&owner,&wrong)).is_err());
    let mut wrong_revision=flow.clone();wrong_revision["expected_account_revision"]=json!(1);
    assert!(service.cancel(&native_identity(&owner,&wrong_revision)).is_err());
    native_wait(&service,&identity,"awaiting_authorization");
    assert_eq!(service.cancel(&identity).unwrap()["phase"],"cancelled");
    assert!(service.finish(&identity).is_err());
    drop(service);
    assert_eq!(f.count("/token"),0);
    assert!(f.host.snapshot(&account).unwrap().is_none());
}

#[test]
fn model_siwc_native_declined_callback_reports_reason_without_account_or_token_exchange() {
    let f = Fixture::new();
    let mut service = native_service(&f);
    let account = uuid::Uuid::new_v4().to_string();
    let owner = service.owner_id().to_string();
    let flow = service.start(&uuid::Uuid::new_v4().to_string(), serde_json::from_value(json!({
        "owner_id":owner,"account_id":account,"expected_account_revision":null,
        "display_name":"Declined fixture","acknowledge_authorization":true
    })).unwrap()).unwrap();
    let identity = native_identity(&owner, &flow);
    let awaiting = native_wait(&service, &identity, "awaiting_authorization");
    let authorization = reqwest::Url::parse(awaiting["authorization_url"].as_str().unwrap()).unwrap();
    let fields: BTreeMap<_,_> = authorization.query_pairs().into_owned().collect();
    let mut callback = reqwest::Url::parse(&fields["redirect_uri"]).unwrap();
    callback.query_pairs_mut().append_pair("state", &fields["state"])
        .append_pair("error", "access_denied")
        .append_pair("error_description", "private-description-must-not-escape");
    reqwest::blocking::Client::builder().no_proxy().timeout(Duration::from_secs(5)).build().unwrap()
        .get(callback).send().unwrap();
    let failed = native_wait(&service, &identity, "failed");
    assert_eq!(failed["error"], "native_siwc_authorization_declined");
    assert_eq!(failed["private_session_may_be_retained"], false);
    assert_eq!(failed["inference_performed"], false);
    assert!(failed["authorization_url"].is_null());
    assert!(!failed.to_string().contains("private-description"));
    assert!(service.finish(&identity).is_err());
    let store = crate::store::NolaStore::open(&f.host.database).unwrap();
    assert!(crate::model_siwc_cli::load(&store, &account).unwrap().is_none());
    assert!(f.host.snapshot(&account).unwrap().is_none());
    assert_eq!(f.count("/token"), 0);
    assert_eq!(f.count("/v1/models"), 0);
    assert_eq!(f.count("/v1/responses"), 0);
}

#[test]
fn model_siwc_native_finish_commits_exact_core_owner_once_and_rejects_stale_account() {
    for scenario in ["commit","conflict","cancel"] {
        let f=Fixture::new();let mut service=native_service(&f);let owner=service.owner_id().to_string();
        let account=uuid::Uuid::new_v4().to_string();
        let flow=service.start(&uuid::Uuid::new_v4().to_string(),serde_json::from_value(json!({
            "owner_id":owner,"account_id":account,"expected_account_revision":null,
            "display_name":"Native fixture","acknowledge_authorization":true})).unwrap()).unwrap();
        let identity=native_identity(&owner,&flow);native_authorize(&f,&service,&identity);
        let store=crate::store::NolaStore::open(&f.host.database).unwrap();
        assert!(crate::model_siwc_cli::load(&store,&account).unwrap().is_none());
        if scenario=="cancel" {
            let cancelled=service.cancel(&identity).unwrap();
            assert_eq!(cancelled["phase"],"cancelled");
            assert_eq!(cancelled["private_session_may_be_retained"],true);
            assert!(service.finish(&identity).is_err());
            assert!(crate::model_siwc_cli::load(&store,&account).unwrap().is_none());
            assert!(f.host.snapshot(&account).unwrap().unwrap().signed_in);
        } else if scenario=="conflict" {
            let session=f.host.snapshot(&account).unwrap().unwrap();
            crate::model_siwc_cli::save_account(&store,&account,"Concurrent owner",None,&session).unwrap();
            assert!(service.finish(&identity).is_err());
            assert_eq!(crate::model_siwc_cli::load(&store,&account).unwrap().unwrap().account.display_name,"Concurrent owner");
            assert!(f.host.snapshot(&account).unwrap().unwrap().signed_in);
        } else {
            let first=service.finish(&identity).unwrap();let second=service.finish(&identity).unwrap();
            assert_eq!(first["phase"],"committed");assert_eq!(first["account"],second["account"]);
            assert_eq!(first["account"]["account"]["id"],account);
            assert_eq!(first["account"]["account"]["source"],"nola_chatgpt");
            assert!(!first.to_string().contains("access-one"));
        }
        assert_eq!(f.count("/token"),1);
        assert_eq!(f.count("/v1/responses"),0);
    }
}

#[test]
fn model_siwc_native_management_keeps_both_revisions_and_explicit_signout() {
    let f=Fixture::new();let id=uuid::Uuid::new_v4().to_string();let signed=f.sign_in(&id);
    let service=native_service(&f);
    let store=crate::store::NolaStore::open(&f.host.database).unwrap();
    let owner=crate::model_siwc_cli::save_account(&store,&id,"Native account",None,&signed).unwrap();
    let request=|core:i64,session:Option<i64>,confirm:bool| serde_json::from_value(json!({
        "owner_id":service.owner_id(),"account_id":id,"expected_account_revision":core,
        "expected_session_revision":session,"acknowledge_sign_out":confirm})).unwrap();
    let count=f.state.lock().unwrap()["requests"].as_array().unwrap().len();
    let status=service.manage("account_status",request(owner.revision,None,false)).unwrap();
    assert_eq!(status["session"]["session_revision"],signed.session_revision);
    assert_eq!(count,f.state.lock().unwrap()["requests"].as_array().unwrap().len());
    assert!(service.manage("models",request(owner.revision+1,Some(signed.session_revision),false)).is_err());
    assert!(service.manage("models",request(owner.revision,Some(signed.session_revision+1),false)).is_err());
    assert_eq!(f.count("/v1/models"),0);
    let models=service.manage("models",request(owner.revision,Some(signed.session_revision),false)).unwrap();
    assert_eq!(models["connection"]["document"]["catalog"]["models"][0]["model_id"],"exact:slug");
    assert!(models["connection"]["document"]["catalog"]["models"][0]["execution_capabilities"].is_null());
    assert!(store.model_profiles().unwrap().iter().all(|profile|!profile.enabled));
    let refreshed=service.manage("refresh",request(owner.revision,Some(signed.session_revision),false)).unwrap();
    let core=refreshed["account"]["revision"].as_i64().unwrap();
    let session=refreshed["session"]["session_revision"].as_i64().unwrap();
    assert!(core>owner.revision && session>signed.session_revision);
    assert!(service.manage("refresh",request(owner.revision,Some(signed.session_revision),false)).is_err());
    assert_eq!(f.count("/token"),2);
    assert!(service.manage("sign_out",request(core,Some(session),false)).is_err());
    assert_eq!(f.count("/revoke"),0);
    f.state.lock().unwrap()["revoke_failure"]=json!(true);
    let signed_out=service.manage("sign_out",request(core,Some(session),true)).unwrap();
    assert_eq!(signed_out["session"]["revocation_unconfirmed"],true);
    assert_eq!(signed_out["account"]["account"]["state"],"disconnected");
    assert!(!signed_out.to_string().contains("access-two"));
    assert_eq!(f.count("/v1/responses"),0);
}

#[test]
fn model_siwc_execution_binding_checks_core_and_private_identity() {
    let f=Fixture::new();let id=uuid::Uuid::new_v4().to_string();f.sign_in(&id);
    let store=crate::store::NolaStore::open(&f.host.database).unwrap();
    let input=serde_json::from_value(json!({"id":id,"display_name":"fixture","source":"nola_chatgpt","credential_ref":format!("siwc:{id}"),"state":"ready"})).unwrap();
    let saved:nola_domain::model_connection::ModelAccountSnapshot=store.application().unwrap().execute(nola_application::ApplicationCommand::Model {command:nola_application::ModelCommand::SaveAccount {input,expected_revision:None,updated_at_ms:1}}).unwrap();
    let binding=f.host.execution_binding(&id,saved.revision,&format!("siwc:{id}")).unwrap();
    assert!(f.host.execution_binding(&id,saved.revision+1,&binding.credential_ref).is_err());
    assert!(f.host.execution_binding(&id,saved.revision,"codex:foreign").is_err());
    let credential=f.host.resolve_execution_credential(&binding).unwrap();
    assert_eq!(credential.bearer(),"access-one");assert_eq!(credential.resource_url(),f.host.endpoints.resource);
    assert!(f.host.lock(&id).is_err());drop(credential);
    let mut changed=binding.clone();changed.subject="different-subject".into();
    assert!(f.host.resolve_execution_credential(&changed).is_err());
    let mut input=saved.account;input.state=nola_domain::model_connection::ModelAccountState::Disconnected;
    let _:nola_domain::model_connection::ModelAccountSnapshot=store.application().unwrap().execute(nola_application::ApplicationCommand::Model {command:nola_application::ModelCommand::SaveAccount {input,expected_revision:Some(saved.revision),updated_at_ms:2}}).unwrap();
    assert!(f.host.resolve_execution_credential(&binding).is_err());
    assert_eq!(f.count("/v1/responses"),0);
}

#[test]
fn model_siwc_fake_http_sign_in_catalog_text_rotation_and_sign_out() {
    let f=Fixture::new();let account=uuid::Uuid::new_v4().to_string();let s=f.sign_in(&account);assert!(s.plan_usage_authorized);
    let public=serde_json::to_string(&s).unwrap();assert!(!public.contains("access-one")&&!public.contains("refresh-one"));
    let choices=f.host.models(&account,s.session_revision).unwrap();assert_eq!(choices.len(),1);assert_eq!(choices[0].slug,"exact:slug");
    let result=f.host.infer_text(&account,s.session_revision,"exact:slug",&[json!({"role":"user","content":"fixture only"})],|_|Ok(())).unwrap();
    assert_eq!(result.state,"completed");assert_eq!(result.usage.unwrap()["input_tokens"],4);
    let refreshed=f.host.refresh(&account,s.session_revision).unwrap();assert!(refreshed.session_revision>s.session_revision);
    assert_eq!(f.host.read(&account).unwrap().1.unwrap().tokens.unwrap().refresh_token,"refresh-two");
    let out=f.host.sign_out(&account,refreshed.session_revision).unwrap();assert!(!out.signed_in);assert!(!out.revocation_unconfirmed);
    let a=f.begin(&account,out.session_revision);let params:BTreeMap<_,_>=reqwest::Url::parse(a.authorization_url()).unwrap().query_pairs().into_owned().collect();
    assert_eq!(params["client_id"],"oaiapp_fixture");assert!(!params.contains_key("agent_name_hint"));
    assert_eq!(f.count("/v1/responses"),1);assert_eq!(f.count("/revoke"),1);assert!(f.host.models(&account,out.session_revision).is_err());
}

#[test]
fn model_siwc_catalog_renews_expired_access_for_the_same_registration() {
    let f = Fixture::new();
    let account = uuid::Uuid::new_v4().to_string();
    let signed_in = f.sign_in(&account);
    f.state.lock().unwrap()["reasoning_levels"] = json!([
        {"effort":"low","description":"Faster"}, {"effort":"high","description":"Deeper"},
        {"effort":"ultra","description":"Multi-agent mode"}
    ]);
    f.state.lock().unwrap()["summary_supported"] = json!(false);
    let (revision, session) = f.host.read(&account).unwrap();
    let mut session = session.unwrap();
    session.tokens.as_mut().unwrap().expires_at = chrono::Utc::now().timestamp() - 1;
    let expired_revision = f.host.write(&account, revision, &session).unwrap();
    assert!(f.host.models(&account, signed_in.session_revision).is_err());
    let service = native_service(&f);
    let store = crate::store::NolaStore::open(&f.host.database).unwrap();
    let owner = crate::model_siwc_cli::save_account(&store, &account, "Expired fixture", None,
        &f.host.snapshot(&account).unwrap().unwrap()).unwrap();
    let result = service.manage("models", serde_json::from_value(json!({
        "owner_id":service.owner_id(),"account_id":account,"expected_account_revision":owner.revision,
        "expected_session_revision":expired_revision,"acknowledge_sign_out":false
    })).unwrap()).unwrap();
    assert_eq!(result["connection"]["document"]["catalog"]["models"][0]["model_id"], "exact:slug");
    assert_eq!(result["connection"]["document"]["catalog"]["models"][0]["reasoning_levels"], json!(["low", "high", "ultra"]));
    assert!(store.model_profiles().unwrap().iter().all(|profile| !profile.enabled));
    let renewed = f.host.snapshot(&account).unwrap().unwrap();
    assert_eq!(result["session"]["session_revision"], renewed.session_revision);
    assert_eq!(renewed.client_id, signed_in.client_id);
    assert_eq!(renewed.subject, signed_in.subject);
    assert!(renewed.session_revision > expired_revision);
    assert_eq!(f.count("/token"), 2);
    assert_eq!(f.count("/v1/models"), 1);
    assert_eq!(f.count("/v1/responses"), 0);
    let connection_id = result["connection"]["document"]["connection"]["id"].as_str().unwrap();
    let _: Value = store.application().unwrap().execute(nola_application::ApplicationCommand::Model {
        command:nola_application::ModelCommand::ConfigureConnectionModels {
            connection_id:connection_id.into(),
            expected_revision:result["connection"]["revision"].as_i64().unwrap(),
            enabled_model_ids:vec!["siwc_responses:exact:slug".into()],
            selected_model_id:None,updated_at_ms:chrono::Utc::now().timestamp_millis(),
        }
    }).unwrap();
    let profile = store.model_profiles().unwrap().into_iter().find(|p|p.enabled).unwrap();
    let source: Value = serde_json::from_str(&profile.source_json).unwrap();
    assert_eq!(source["reasoning"], json!({"control":"effort","efforts":["low","high"]}));
    assert_eq!(source["executionCapabilities"]["reasoning"], true);
    assert_eq!(source["reasoningSummarySupported"], false);
    let binding = f.host.execution_binding(&account, owner.revision,
        owner.account.credential_ref.as_deref().unwrap()).unwrap();
    let credential = f.host.resolve_execution_credential(&binding).unwrap();
    for (tier, effort) in [("concise", "low"), ("balanced", "low"), ("detailed", "high")] {
        let settings = crate::model_profile_resolution::resolve_remote_profile_settings(
            &profile, &json!({"reasoning_tier":tier}), &Default::default(), &[]).unwrap();
        assert_eq!(settings.selected_profile.as_ref().unwrap().profile_id, profile.id);
        assert_eq!(settings.selected_profile.as_ref().unwrap().admitted_revision, profile.revision);
        let config = crate::llm::LlmConfig {
            siwc: None, native_model_route: None, provider: settings.runtime.provider.clone(),
            fallbacks: vec![], endpoint: settings.endpoint, api_key: String::new(), model: settings.model,
            temperature: settings.temperature, top_p: settings.top_p,
            max_output_tokens: settings.max_output_tokens, compaction_output_budget: None,
            chat_output_token_field: settings.runtime.chat_output_token_field, max_retries: 0,
            parallel_tool_calls: None, thinking_enabled: true,
            reasoning_summary_supported: settings.runtime.model_capabilities.reasoning_summary_supported,
            reasoning_selection: settings.runtime.reasoning_selection,
            sampling_override: settings.runtime.sampling_override,
            generation_options: settings.runtime.generation_options, cancellation: Default::default(),
            openai_prompt_cache: false, openai_prompt_cache_key: None,
            openai_prompt_cache_key_prefix: None, openai_prompt_cache_retention: None,
            openai_prompt_cache_history_breakpoint: false,
            openai_prompt_cache_history_breakpoint_endpoints: None,
        };
        let wire = crate::llm::siwc_request_body_for_test(&config).unwrap();
        assert_eq!(wire["model"], "exact:slug");
        assert_eq!(wire["reasoning"], json!({"effort":effort}));
        assert!(wire.get("max_output_tokens").is_none());
        f.state.lock().unwrap()["expected_response_body"] = wire.clone();
        f.state.lock().unwrap()["expected_bearer"] = json!(format!("Bearer {}", credential.bearer()));
        // Only the local fixture receives this request; the fixed official
        // production transport and its credential checks remain unchanged.
        let response = f.host.http.post(format!("{}/responses", credential.resource_url()))
            .bearer_auth(credential.bearer()).json(&wire).send().unwrap();
        assert!(response.status().is_success());
        assert!(response.text().unwrap().contains("response.completed"));
    }
    assert_eq!(f.count("/v1/responses"), 3);
    assert!(crate::model_profile_resolution::resolve_remote_profile_settings(&profile,
        &json!({"reasoning_selection":{"kind":"effort","value":"ultra"}}),
        &Default::default(), &[]).is_err());
}

#[test]
fn model_siwc_state_pkce_scope_and_identity_are_not_source_login() {
    let f=Fixture::new();let account=uuid::Uuid::new_v4().to_string();let a=f.begin(&account,0);
    let params:BTreeMap<_,_>=reqwest::Url::parse(a.authorization_url()).unwrap().query_pairs().into_owned().collect();
    assert_eq!(params["client_id"],DYNAMIC_CLIENT);assert_eq!(params["agent_name_hint"],"Nola");assert_eq!(params["code_challenge"],URL_SAFE_NO_PAD.encode(Sha256::digest(a.verifier.as_bytes())));
    for path in ["/auth/callback?state=wrong&code=x&client_id=oaiapp_fixture".into(),format!("/auth/callback?state={}&state={}&code=x&client_id=oaiapp_fixture",a.state,a.state),format!("/auth/callback?state={}&error=access_denied",a.state)] {assert!(f.host.exchange_callback(&a,&path).is_err());}
    assert_eq!(f.count("/token"),0);f.state.lock().unwrap()["scope"]=json!("openid profile email offline_access");
    let s=f.host.exchange_callback(&a,&callback(&a)).unwrap();assert!(s.signed_in);assert!(!s.plan_usage_authorized);assert!(f.host.models(&account,s.session_revision).is_err());
    assert!(f.host.exchange_callback(&a,&callback(&a)).is_err()); // Accepted revision cannot be replayed.
    let returning=f.begin(&account,s.session_revision);f.state.lock().unwrap()["subject"]=json!("different-subject-same-email");
    assert!(f.host.exchange_callback(&returning,&callback(&returning)).is_err());assert_eq!(f.host.snapshot(&account).unwrap().unwrap().subject,"subject-one");
}

#[test]
fn model_siwc_refresh_unknown_is_durable_and_revocation_failure_is_honest() {
    let f=Fixture::new();let account=uuid::Uuid::new_v4().to_string();let s=f.sign_in(&account);
    let lock=f.host.lock(&account).unwrap();assert!(f.host.refresh(&account,s.session_revision).is_err());drop(lock);
    f.state.lock().unwrap()["refresh_failure"]=json!(true);assert!(f.host.refresh(&account,s.session_revision).is_err());
    let failed=f.host.snapshot(&account).unwrap().unwrap();assert!(failed.reauthorization_required);assert!(!failed.plan_usage_authorized);
    let mut reopened=SiwcHost::open(&f.host.database).unwrap();reopened.endpoints=f.host.endpoints.clone();assert!(reopened.refresh(&account,failed.session_revision).is_err());assert_eq!(f.count("/token"),2);
    f.state.lock().unwrap()["revoke_failure"]=json!(true);let out=reopened.sign_out(&account,failed.session_revision).unwrap();assert!(!out.signed_in);assert!(out.revocation_unconfirmed);assert!(reopened.models(&account,out.session_revision).is_err());
}

#[test]
fn model_siwc_jwt_signature_nonce_audience_and_expiry_are_verified() {
    let f=Fixture::new();let d=f.host.discovery().unwrap();let now=chrono::Utc::now().timestamp();
    let valid=json!({"iss":d.issuer,"aud":"oaiapp_fixture","sub":"s","iat":now,"exp":now+60,"nonce":"expected"});
    let token=signed(&valid);assert!(f.host.verify_identity(&d,&token,"oaiapp_fixture",Some("expected")).is_ok());
    for (field,value) in [("iss",json!("https://other.invalid")),("aud",json!("other-client")),("nonce",json!("wrong")),("exp",json!(now-60)),("iat",json!(now+60)),("sub",json!(""))] {
        let mut bad=valid.clone();bad[field]=value;assert!(f.host.verify_identity(&d,&signed(&bad),"oaiapp_fixture",Some("expected")).is_err(),"{field}");
    }
    let mut parts:Vec<_>=token.split('.').map(str::to_owned).collect();let replacement=if parts[2].starts_with('A'){"B"}else{"A"};parts[2].replace_range(0..1,replacement);
    assert!(f.host.verify_identity(&d,&parts.join("."),"oaiapp_fixture",Some("expected")).is_err());
}

#[test]
fn model_siwc_stream_needs_real_terminal_and_keeps_unknown_usage() {
    assert!(consume_stream(&b"data: {\"type\":\"response.output_text.delta\",\"delta\":\"x\"}\n\n"[..],&mut |_|Ok(())).is_err());
    for (kind,state) in [("response.failed","failed"),("response.incomplete","incomplete"),("response.completed","completed")] {
        let stream=format!("data: {{\"type\":\"{kind}\",\"response\":{{\"id\":\"r\"}}}}\n\n");let result=consume_stream(stream.as_bytes(),&mut |_|Ok(())).unwrap();assert_eq!(result.state,state);assert!(result.usage.is_none());
    }
}

#[test]
fn model_siwc_cancel_identity_only_and_repeated_sign_out() {
    let f=Fixture::new();let account=uuid::Uuid::new_v4().to_string();let a=f.begin(&account,0);
    a.cancellation().cancel();assert!(f.host.finish_sign_in(a).is_err());assert_eq!(f.count("/token"),0);
    let tokens=token_set(TokenResponse {access_token:String::new(),refresh_token:String::new(),id_token:Some("verified-before-storage".into()),token_type:"Bearer".into(),expires_in:0,scope:"openid profile email".into()},None).unwrap();
    assert!(!plan_scopes(&tokens.scopes));
    let s=f.sign_in(&account);f.state.lock().unwrap()["revoke_failure"]=json!(true);
    let first=f.host.sign_out(&account,s.session_revision).unwrap();let second=f.host.sign_out(&account,first.session_revision).unwrap();
    assert!(first.revocation_unconfirmed&&second.revocation_unconfirmed);assert_eq!(f.count("/revoke"),1);
}

#[test]
fn model_siwc_callback_listener_uses_exact_loopback_attempt() {
    let f=Fixture::new();let account=uuid::Uuid::new_v4().to_string();let a=f.begin(&account,0);
    let address=a.listener.local_addr().unwrap();let request=format!("GET {} HTTP/1.1\r\nHost: {address}\r\n\r\n",callback(&a));
    let browser=std::thread::spawn(move || {
        let mut connection=std::net::TcpStream::connect(address).unwrap();connection.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        connection.write_all(request.as_bytes()).unwrap();let mut reply=String::new();connection.read_to_string(&mut reply).unwrap();reply
    });
    assert!(f.host.finish_sign_in(a).unwrap().plan_usage_authorized);assert!(browser.join().unwrap().contains("Nola sign-in completed"));
}
