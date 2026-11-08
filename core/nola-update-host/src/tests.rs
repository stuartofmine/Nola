use super::*;
use ring::signature::{Ed25519KeyPair, KeyPair};
use std::{cell::RefCell, io::Cursor};

fn signing_key() -> Ed25519KeyPair { Ed25519KeyPair::from_seed_unchecked(&[42;32]).unwrap() }
fn source() -> UpdateSource {
    UpdateSource {schema_version:1,version:"1.0.0".into(),build:1,channel:"internal".into(),
        feed_url:Some("https://updates.example.org/feed.json".into()),public_key:Some(STANDARD.encode(signing_key().public_key().as_ref()))}
}
fn manifest() -> Manifest {
    let time=now().unwrap();
    Manifest {schema_version:1,channel:"internal".into(),version:"1.0.1".into(),build:2,issued_at:time-60,expires_at:time+3600,
        notes:"A release note with <literal> text".into(),store_schema:nola_store::STORE_SCHEMA_VERSION,store_epoch:nola_store::STORE_SCHEMA_EPOCH,targets:vec![TargetRelease {target:"cli-macos-aarch64".into(),artifact:Artifact::Archive {
            url:"https://updates.example.org/Nola.zip".into(),filename:"Nola.zip".into(),size:7,sha256:hex::encode(Sha256::digest(b"package"))}}]}
}
fn envelope(value:&Manifest)->Vec<u8> {
    let payload=serde_json::to_vec(value).unwrap();
    serde_json::to_vec(&serde_json::json!({"payload_base64":STANDARD.encode(&payload),"signature_base64":STANDARD.encode(signing_key().sign(&payload).as_ref())})).unwrap()
}
struct Fake {feed:Vec<u8>,package:Vec<u8>, calls:RefCell<Vec<String>>}
impl Fake {fn new(value:&Manifest)->Self {Self{feed:envelope(value),package:b"package".to_vec(),calls:RefCell::new(vec![])}}}
impl Transport for Fake {
    fn get(&self,url:&Url)->Result<Box<dyn Read>> {
        self.calls.borrow_mut().push(url.to_string());
        Ok(Box::new(Cursor::new(if url.path().ends_with("feed.json"){self.feed.clone()}else{self.package.clone()})))
    }
}
#[test] fn disabled_is_not_latest_and_never_networks() {
    let mut source=source(); source.feed_url=None; source.public_key=None;
    let transport=Fake::new(&manifest());
    assert_eq!(check(&source,"ios",&transport).unwrap().status,"disabled");assert!(transport.calls.borrow().is_empty());
}
#[test] fn newer_current_ahead_and_missing_platform_are_distinct() {
    let transport=Fake::new(&manifest());let mut source=source();
    assert_eq!(check(&source,"cli-macos-aarch64",&transport).unwrap().status,"available");
    assert_eq!(check(&source,"ios",&transport).unwrap().status,"unavailable");
    source.version="1.0.1".into();source.build=2;
    assert_eq!(check(&source,"ios",&transport).unwrap().status,"unavailable");
    assert_eq!(check(&source,"cli-macos-aarch64",&transport).unwrap().status,"up_to_date");
    source.build=3;assert_eq!(check(&source,"ios",&transport).unwrap().status,"ahead");
}
#[test] fn unsigned_modified_and_wrong_key_fail_closed() {
    let mut transport=Fake::new(&manifest());
    let mut value:serde_json::Value=serde_json::from_slice(&transport.feed).unwrap();
    value["payload_base64"]=STANDARD.encode(b"{}").into();transport.feed=serde_json::to_vec(&value).unwrap();
    assert_eq!(check(&source(),"ios",&transport).unwrap_err().code,"signature_invalid");
    let mut source=source();source.public_key=Some(STANDARD.encode([0;32]));
    assert_eq!(check(&source,"ios",&Fake::new(&manifest())).unwrap_err().code,"signature_invalid");
}
#[test] fn expired_future_wrong_channel_unknown_format_and_duplicate_target_fail() {
    for kind in 0..6 {
        let mut manifest=manifest();let time=now().unwrap();
        match kind {0=>manifest.expires_at=time,1=>{manifest.issued_at=time+301;manifest.expires_at=time+3600;},
            2=>manifest.channel="stable".into(),3=>manifest.schema_version=2,
            4=>manifest.targets.push(manifest.targets[0].clone()),_=>manifest.expires_at=time+MAX_LIFETIME+1}
        assert!(check(&source(),"ios",&Fake::new(&manifest)).is_err(),"kind {kind}");
    }
}
#[test] fn downgrade_does_not_offer_and_same_build_conflict_is_error() {
    let mut manifest=manifest();manifest.build=1;
    assert_eq!(check(&source(),"ios",&Fake::new(&manifest)).unwrap_err().code,"identity_conflict");
    manifest.version="1.0.0".into();let mut source=source();source.build=2;
    assert_eq!(check(&source,"ios",&Fake::new(&manifest)).unwrap().status,"ahead");
}
#[test] fn source_rejects_incomplete_credentials_http_and_unknown_fields() {
    let source=source();
    for change in [serde_json::json!({"feed_url":"http://example.com/feed"}),serde_json::json!({"public_key":null}),
        serde_json::json!({"feed_url":"https://user:secret@example.com/feed"}),serde_json::json!({"legacy_source":true})] {
        let mut value=serde_json::to_value(&source).unwrap();for (key,value_)in change.as_object().unwrap(){value[key]=value_.clone();}
        assert!(UpdateSource::parse(&value.to_string()).is_err());
    }
}
#[test] fn invalid_artifact_path_size_digest_scheme_and_target_rejected() {
    for kind in 0..6 {
        let mut manifest=manifest();
        if let Artifact::Archive {url,filename,size,sha256}=&mut manifest.targets[0].artifact {
            match kind {0=>*filename="../Nola.zip".into(),1=>*url="file:///tmp/package".into(),2=>*size=MAX_PACKAGE+1,
                3=>*sha256="f".repeat(63),4=>*filename=".hidden.zip".into(),_=>*filename="Nola.exe".into()}
        }
        let code = check(&source(),"ios",&Fake::new(&manifest)).unwrap_err().code;
        assert_eq!(code, if kind == 1 {"url_invalid"} else {"artifact_invalid"}, "kind {kind}");
    }
}
#[test] fn store_targets_cannot_receive_executables_or_arbitrary_links() {
    for (target,url,valid)in [("ios","https://apps.apple.com/app/id123",true),("ios","https://example.com/app",false),
        ("android-store","https://play.google.com/store/apps/details?id=com.nola",true),("android-store","https://apps.apple.com/app/id123",false)] {
        assert_eq!(validate_artifact(target,&Artifact::Store {url:url.into()}).is_ok(),valid);
    }
    assert!(validate_artifact("android-store",&Artifact::Installer{url:"https://updates.example.com/Nola.apk".into(),filename:"Nola.apk".into(),size:7,sha256:"a".repeat(64)}).is_err());
}
#[test] fn verified_download_is_atomic_and_never_overwrites() {
    let temp=tempfile::tempdir().unwrap();let source=source();let transport=Fake::new(&manifest());
    let offer=check(&source,"cli-macos-aarch64",&transport).unwrap().release.unwrap();
    let path=download(&source,"cli-macos-aarch64",&offer.release_id,temp.path(),&transport).unwrap();
    assert_eq!(fs::read(&path).unwrap(),b"package");
    assert!(download(&source,"cli-macos-aarch64",&offer.release_id,temp.path(),&transport).is_err());
    assert_eq!(fs::read(path).unwrap(),b"package");assert_eq!(fs::read_dir(temp.path()).unwrap().count(),1);
}
#[test] fn corrupt_short_large_packages_are_removed_and_not_published() {
    for package in [b"corrupt".to_vec(),b"short".to_vec(),b"much too long".to_vec()] {
        let temp=tempfile::tempdir().unwrap();let source=source();let mut transport=Fake::new(&manifest());transport.package=package;
        let offer=check(&source,"cli-macos-aarch64",&transport).unwrap().release.unwrap();
        assert_eq!(download(&source,"cli-macos-aarch64",&offer.release_id,temp.path(),&transport).unwrap_err().code,"package_invalid");
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(),0);
    }
}
#[test] fn changed_publication_does_not_download_any_package() {
    let temp=tempfile::tempdir().unwrap();let transport=Fake::new(&manifest());
    assert_eq!(download(&source(),"cli-macos-aarch64",&"0".repeat(64),temp.path(),&transport).unwrap_err().code,"release_changed");
    assert_eq!(transport.calls.borrow().len(),1);assert_eq!(fs::read_dir(temp.path()).unwrap().count(),0);
}
#[test] fn unknown_fields_and_operations_are_rejected_without_model_or_store() {
    for request in ["{}",r#"{"operation":"check","target":"ios","api_key":"never"}"#,r#"{"operation":"install","target":"ios"}"#] {
        assert_eq!(dispatch(request)["ok"],false);
    }
    let mut disabled=source(); disabled.feed_url=None; disabled.public_key=None;
    assert_eq!(dispatch_with_source(r#"{"operation":"check","target":"ios"}"#, &disabled)["value"]["status"],"disabled");
}

#[test] fn changed_storage_contract_blocks_offer_without_opening_or_converting_data() {
    let mut manifest=manifest();manifest.store_schema+=1;
    let checked=check(&source(),"cli-macos-aarch64",&Fake::new(&manifest)).unwrap();
    assert_eq!(checked.status,"incompatible_data");assert!(checked.release.is_none());
}

#[test] fn redirects_allow_github_cdn_without_allowing_downgrades_credentials_or_other_hosts() {
    for (previous,next,allowed) in [
        ("https://github.com/stuartofmine/Nola/releases/latest/download/feed.json","https://github.com/stuartofmine/Nola/releases/download/v1.0.0/feed.json",true),
        ("https://github.com/stuartofmine/Nola/releases/download/v1.0.0/feed.json","https://release-assets.githubusercontent.com/asset?token=public-download",true),
        ("https://release-assets.githubusercontent.com/one","https://release-assets.githubusercontent.com/two",true),
        ("https://github.com/a","http://github.com/b",false),
        ("https://github.com/a","https://evil.example/asset",false),
        ("https://github.com/a","https://release-assets.githubusercontent.com.evil.example/asset",false),
        ("https://evil.example/a","https://release-assets.githubusercontent.com/asset",false),
        ("https://github.com/a","https://user:secret@release-assets.githubusercontent.com/asset",false),
        ("https://github.com/a","https://release-assets.githubusercontent.com:444/asset",false),
    ] { assert_eq!(update_redirect_allowed(&Url::parse(previous).unwrap(),&Url::parse(next).unwrap()),allowed); }
}
