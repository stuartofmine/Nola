// SPDX-License-Identifier: Apache-2.0
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args=std::env::args().skip(1).collect::<Vec<_>>();
    if args.len()!=3 {return Err("Usage: verify_feed <public-source.json> <signed-feed.json> <target>".into());}
    let source=nola_update_host::UpdateSource::parse(&std::fs::read_to_string(&args[0])?)?;
    let now=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs();
    let (manifest,id)=nola_update_host::verify_feed(&source,&std::fs::read(&args[1])?,&args[2],now)?;
    if manifest.version != source.version || manifest.build != source.build {
        return Err("Publisher source and manifest release identities differ".into());
    }
    if manifest.store_schema != nola_store::STORE_SCHEMA_VERSION || manifest.store_epoch != nola_store::STORE_SCHEMA_EPOCH {
        return Err("Published data format does not match this release's Store contract".into());
    }
    if !manifest.targets.iter().any(|target|target.target==args[2]) {return Err("Required release target missing".into());}
    println!("Verified {} ({}) {}",manifest.version,manifest.build,id);Ok(())
}
