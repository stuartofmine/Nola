// SPDX-License-Identifier: Apache-2.0
// Updates use a publisher-pinned public source, never browser-supplied URLs,
// signing keys, directories, shell commands, or model configuration.
fn software_update_target() -> String {
    if cfg!(target_os = "windows") {
        let value = std::env::var("NOLA_UPDATE_TARGET").unwrap_or_else(|_| nola_update_host::native_target("desktop"));
        if value.starts_with(&format!("windows-{}-", std::env::consts::ARCH)) { return value; }
        return "unsupported".into();
    }
    nola_update_host::native_target("cli")
}
struct VerifiedUpdateDownload { file: fs::File, _directory: tempfile::TempDir }
impl Read for VerifiedUpdateDownload {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> { self.file.read(output) }
}
fn software_update_response(request: &mut Request, method: &Method, path: &str) -> Response<Box<dyn Read + Send>> {
    let source = match nola_update_host::UpdateSource::bundled() {
        Ok(source) => source,
        Err(error) => return json_response(StatusCode(503), json!({"error":error.message,"code":error.code})),
    };
    software_update_response_for_source(request, method, path, &source)
}
fn software_update_response_for_source(request: &mut Request, method: &Method, path: &str,
    source: &nola_update_host::UpdateSource) -> Response<Box<dyn Read + Send>> {
    let target = software_update_target();
    if path == "/api/software-update" {
        if method != &Method::Get { return method_not_allowed(); }
        return json_response(StatusCode(200), json!({"current_version":source.version,"current_build":source.build,
            "channel":source.channel,"target":target,"status":if source.feed_url.is_some(){"not_checked"}else{"disabled"},"release":null}));
    }
    let package_request = path == "/api/software-update/package";
    if method != if package_request { &Method::Get } else { &Method::Post } { return method_not_allowed(); }
    // Native same-origin navigation streams a verified package directly to the
    // browser download manager, avoiding a multi-gigabyte JavaScript Blob.
    let same_origin_navigation = package_request && request_header(request, "Sec-Fetch-Site") == Some("same-origin");
    if !same_origin_navigation && !web_request_is_same_origin(request) { return json_response(StatusCode(403), json!({"error":"same_origin_required"})); }
    if !package_request {
        #[derive(Deserialize)] #[serde(deny_unknown_fields)] struct CheckRequest {}
        if let Err(response) = read_json::<CheckRequest>(request) { return response; }
        return match nola_update_host::check(&source, &target, &nola_update_host::HttpsTransport) {
            Ok(value) => json_response(StatusCode(200), serde_json::to_value(value).unwrap()),
            Err(error) => json_response(StatusCode(502), json!({"error":error.message,"code":error.code})),
        };
    }
    let query = request.url().split_once('?').map(|(_,query)|query).unwrap_or("");
    let fields = url::form_urlencoded::parse(query.as_bytes()).collect::<Vec<_>>();
    if fields.len() != 1 || fields[0].0 != "release_id" || fields[0].1.len() != 64 || !fields[0].1.bytes().all(|b| b.is_ascii_hexdigit()) {
        return json_response(StatusCode(400),json!({"error":"invalid_release_identity"}));
    }
    let release_id = fields[0].1.to_string();
    let result = (|| -> Result<Response<Box<dyn Read + Send>>, String> {
        let directory = tempfile::tempdir().map_err(|e|e.to_string())?;
        let package = nola_update_host::download(&source, &target, &release_id, directory.path(), &nola_update_host::HttpsTransport).map_err(|e|e.to_string())?;
        let file = fs::File::open(&package).map_err(|e|e.to_string())?;
        let length = file.metadata().map_err(|e|e.to_string())?.len();
        let name = package.file_name().and_then(|v|v.to_str()).ok_or("Invalid update package filename")?;
        let headers = vec![Header::from_bytes("Content-Type", "application/octet-stream").unwrap(),
            Header::from_bytes("Content-Disposition", format!("attachment; filename=\"{name}\"")).unwrap(),
            Header::from_bytes("Cache-Control", "no-store").unwrap()];
        Ok(Response::new(StatusCode(200), headers, Box::new(VerifiedUpdateDownload {file,_directory:directory}) as Box<dyn Read+Send>, usize::try_from(length).ok(), None))
    })();
    result.unwrap_or_else(|error|json_response(StatusCode(502),json!({"error":error})))
}

#[cfg(test)]
mod software_update_web_tests {
    use super::*;
    fn request(method:&str,path:&str,same_origin:bool,body:&str)->(u16,Value) {
        let server=Server::http("127.0.0.1:0").unwrap();
        let origin=format!("http://{}",server.server_addr());
        let worker=std::thread::spawn(move||{
            let mut request=server.recv().unwrap();
            let method=request.method().clone();let url=request.url().to_string();
            let path=url.split('?').next().unwrap();
            let mut source=nola_update_host::UpdateSource::bundled().unwrap();
            source.feed_url=None; source.public_key=None;
            let response=software_update_response_for_source(&mut request,&method,path,&source);
            request.respond(response).unwrap();
        });
        let client=reqwest::blocking::Client::new();
        let mut builder=client.request(reqwest::Method::from_bytes(method.as_bytes()).unwrap(),format!("{origin}{path}"));
        builder=builder.header("Origin",if same_origin{origin.as_str()}else{"https://untrusted.example"});
        let response=builder.body(body.to_string()).send().unwrap();
        let status=response.status().as_u16();let json=response.json::<Value>().unwrap();worker.join().unwrap();(status,json)
    }
    #[test] fn local_status_and_explicit_check_do_not_claim_latest_when_disabled() {
        let (status,value)=request("GET","/api/software-update",true,"");
        assert_eq!(status,200);assert_eq!(value["status"],"disabled");assert!(value.get("feed_url").is_none());
        let (status,value)=request("POST","/api/software-update/check",true,"{}");
        assert_eq!(status,200);assert_eq!(value["status"],"disabled");
    }
    #[test] fn check_and_download_reject_cross_origin_source_injection_and_wrong_methods() {
        assert_eq!(request("POST","/api/software-update/check",false,"{}").0,403);
        assert_eq!(request("POST","/api/software-update/check",true,r#"{"feed_url":"https://untrusted.example"}"#).0,400);
        assert_eq!(request("GET","/api/software-update/check",true,"").0,405);
        assert_eq!(request("GET","/api/software-update/package?release_id=bad",true,"").0,400);
        assert_eq!(request("GET",&format!("/api/software-update/package?release_id={}","a".repeat(64)),false,"").0,403);
    }
}
