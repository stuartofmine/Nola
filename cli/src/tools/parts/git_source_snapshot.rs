// SPDX-License-Identifier: Apache-2.0
// Source-byte snapshots through a private Git index. Never stage/reset user files.

fn source_snapshot_plan_schema()->Value {
    json!({"type":"object","properties":{"source_head":{"type":"string"},"state_sha256":{"type":"string"}},
        "required":["source_head","state_sha256"],"additionalProperties":false})
}
fn task_worktree_selection_schema(require_identity:bool)->Value {
    let mut required=vec!["root","expected_head","expected_common_directory"];
    if require_identity {required.push("expected_directory_identity");}
    json!({"type":"object","description":"Exact witnessed task_worktree_selection returned by creation; not an OS sandbox.","properties":{
        "root":{"type":"string"},"expected_head":{"type":"string"},"expected_common_directory":{"type":"string"},
        "source_snapshot":{"type":"object","properties":{"source_root":{"type":"string"},"plan":source_snapshot_plan_schema(),"tree":{"type":"string"}},
            "required":["source_root","plan","tree"],"additionalProperties":false},
        "expected_directory_identity":{"type":"object","properties":{"root":{"type":"string"},"git_directory":{"type":"string"},"common_directory":{"type":"string"}},
            "required":["root","git_directory","common_directory"],"additionalProperties":false}},
        "required":required,"additionalProperties":false})
}

struct SourceEntry { path:String, mode:u32, sha256:Option<String>, captured:Option<PathBuf> }
struct SourceState {
    plan:nola_domain::task_workspace::SourceSnapshotPlan,
    entries:Vec<SourceEntry>, dirty:bool, untracked:bool,
}

fn source_entry(root:&Path,path:&str,capture:Option<&Path>,cancel:Option<&RuntimeExecutionControl>)->Result<SourceEntry,String> {
    use sha2::{Digest,Sha256};
    let relative=Path::new(path);
    if relative.is_absolute() || relative.components().any(|part|!matches!(part,std::path::Component::Normal(_)))
        || bash_token_is_sensitive_path(path) {
        return Err("Snapshot refuses unsafe or sensitive changed paths".into());
    }
    let full=root.join(relative);
    let parent=resolve_missing_path(full.parent().ok_or("Snapshot path has no parent")?)
        .ok_or("Snapshot parent cannot be resolved")?;
    if !parent.starts_with(root) {return Err("Snapshot path leaves its source checkout".into());}
    let metadata=match fs::symlink_metadata(&full) {
        Ok(value)=>value,
        Err(error) if error.kind()==io::ErrorKind::NotFound=>return Ok(SourceEntry {path:path.into(),mode:0,sha256:None,captured:None}),
        Err(error)=>return Err(error.to_string()),
    };
    let mut hasher=Sha256::new();
    let mut destination=capture.map(|directory| {
        let file=directory.join(uuid::Uuid::new_v4().simple().to_string());
        fs::OpenOptions::new().write(true).create_new(true).open(&file).map(|handle|(file,handle))
    }).transpose().map_err(|error|error.to_string())?;
    let mode=if metadata.file_type().is_symlink() {
        let target=fs::read_link(&full).map_err(|error|error.to_string())?;
        let bytes=target.to_str().ok_or("Non-UTF8 symlink target cannot be snapshotted")?.as_bytes();
        hasher.update(bytes);
        if let Some((_,file))=&mut destination {std::io::Write::write_all(file,bytes).map_err(|error|error.to_string())?;}
        0o120000
    } else if metadata.is_file() {
        if metadata.len()>8*1024*1024 {return Err("Changed file exceeds the bounded snapshot size".into());}
        let mut options=fs::OpenOptions::new(); options.read(true);
        #[cfg(unix)] {use std::os::unix::fs::OpenOptionsExt;options.custom_flags(libc::O_NOFOLLOW);}
        let mut file=options.open(&full).map_err(|error|error.to_string())?;
        let mut buffer=[0u8;64*1024];
        loop {
            if cancel.is_some_and(RuntimeExecutionControl::is_requested) {return Err("Source snapshot cancelled".into());}
            let count=std::io::Read::read(&mut file,&mut buffer).map_err(|error|error.to_string())?;
            if count==0 {break;}
            hasher.update(&buffer[..count]);
            if let Some((_,target))=&mut destination {std::io::Write::write_all(target,&buffer[..count]).map_err(|error|error.to_string())?;}
        }
        #[cfg(unix)] {use std::os::unix::fs::PermissionsExt;if metadata.permissions().mode()&0o111!=0 {0o100755}else{0o100644}}
        #[cfg(not(unix))] {let record=git_command_output(root,&["ls-files","--stage","--",path])?;
            if record.starts_with(b"100755 ") {0o100755}else{0o100644}}
    } else {return Err("Snapshot does not flatten directories, submodules or special files".into());};
    Ok(SourceEntry {path:path.into(),mode,sha256:Some(format!("{:x}",hasher.finalize())),captured:destination.map(|(path,_)|path)})
}

fn source_state(root:&Path,capture:Option<&Path>,cancel:Option<&RuntimeExecutionControl>)->Result<SourceState,String> {
    use sha2::{Digest,Sha256};
    require_nonexecuting_worktree_preparation(root)?;
    let head=git_commit_text(root,&["rev-parse","HEAD"])?;
    let index=git_command_output(root,&["ls-files","--stage","-z"])?;
    if !git_command_output(root,&["ls-files","--unmerged","-z"])?.is_empty() {return Err("Source has unresolved index conflicts".into());}
    let status=git_command_output(root,&["--no-optional-locks","status","--porcelain=v1","-z","--untracked-files=normal","--no-renames"])?;
    let mut entries=Vec::new();let mut untracked=false;
    for record in status.split(|byte|*byte==0).filter(|record|!record.is_empty()) {
        if record.len()<4 || record[2]!=b' ' {return Err("Unsupported source status record".into());}
        if &record[..2]==b"??" {untracked=true;continue;}
        let path=std::str::from_utf8(&record[3..]).map_err(|_|"Non-UTF8 changed path")?;
        entries.push(source_entry(root,path,capture,cancel)?);
    }
    let mut hash=Sha256::new();hash.update(head.as_bytes());hash.update(&index);hash.update(&status);
    for entry in &entries {hash.update(serde_json::to_vec(&(entry.path.as_str(),entry.mode,&entry.sha256)).map_err(|error|error.to_string())?);}
    Ok(SourceState {plan:nola_domain::task_workspace::SourceSnapshotPlan {source_head:head,state_sha256:format!("{:x}",hash.finalize())},dirty:!status.is_empty(),entries,untracked})
}

fn snapshot_tree(root:&Path,state:&SourceState,directory:&Path)->Result<String,String> {
    let index=directory.join("index");
    git_private_index_output(root,&index,&["read-tree",&state.plan.source_head])?;
    let mut updates=Vec::new();
    for entry in &state.entries {
        let object=if let Some(path)=&entry.captured {
            git_commit_text(root,&["hash-object","-w","--no-filters","--",&git_path_argument(path)?])?
        } else {"0".repeat(state.plan.source_head.len())};
        updates.extend_from_slice(format!("{:o} {object}\t{}\0",entry.mode,entry.path).as_bytes());
    }
    let input=directory.join("index-info");fs::write(&input,updates).map_err(|error|error.to_string())?;
    let output=git_operation_command(root).env("GIT_INDEX_FILE",git_path_argument(&index)?).args(["update-index","-z","--index-info"])
        .stdin(Stdio::from(fs::File::open(input).map_err(|error|error.to_string())?)).output().map_err(|error|error.to_string())?;
    if !output.status.success() {return Err("Private source snapshot index preparation failed".into());}
    Ok(String::from_utf8(git_private_index_output(root,&index,&["write-tree"])?).map_err(|error|error.to_string())?.trim().into())
}

fn prepare_source_snapshot(root:&Path,expected:&nola_domain::task_workspace::SourceSnapshotPlan,cancel:&RuntimeExecutionControl)
    ->Result<(String,nola_domain::task_workspace::SourceSnapshotBinding),String> {
    expected.validate()?;
    let directory=tempfile::tempdir().map_err(|error|error.to_string())?;
    let state=source_state(root,Some(directory.path()),Some(cancel))?;
    if state.plan!=*expected || !state.dirty || state.untracked {return Err("Source snapshot changed, is unnecessary, or contains untracked files requiring explicit review".into());}
    let tree=snapshot_tree(root,&state,directory.path())?;
    if source_state(root,None,Some(cancel))?.plan!=*expected {return Err("Source changed during snapshot; no working checkout was overwritten".into());}
    let output=git_operation_command(root).args(["-c","user.name=Nola","-c","user.email=nola@local.invalid","-c","commit.gpgsign=false",
        "commit-tree",&tree,"-p",&expected.source_head,"-m",&format!("Nola task source snapshot {}",expected.state_sha256)])
        .env("GIT_AUTHOR_NAME","Nola").env("GIT_COMMITTER_NAME","Nola").env("GIT_AUTHOR_EMAIL","nola@local.invalid").env("GIT_COMMITTER_EMAIL","nola@local.invalid")
        .env_remove("GIT_AUTHOR_DATE").env_remove("GIT_COMMITTER_DATE").output().map_err(|error|error.to_string())?;
    if !output.status.success() {return Err("Source snapshot commit object could not be written".into());}
    let commit=String::from_utf8(output.stdout).map_err(|error|error.to_string())?.trim().to_owned();
    Ok((commit,nola_domain::task_workspace::SourceSnapshotBinding {source_root:root.to_string_lossy().into_owned(),plan:expected.clone(),tree}))
}

fn verify_source_snapshot(binding:&nola_domain::task_workspace::TaskWorktreeBinding,fresh_source:bool)->Result<(),String> {
    let Some(snapshot)=&binding.selection.source_snapshot else {return Ok(());};
    let root=Path::new(&snapshot.source_root);
    let base=&binding.selection.expected_head;
    if git_commit_text(Path::new(&binding.selection.root),&["rev-parse",&format!("{base}^{{tree}}")])?!=snapshot.tree
        || git_commit_text(Path::new(&binding.selection.root),&["rev-parse",&format!("{base}^")])?!=snapshot.plan.source_head
        || git_commit_text(Path::new(&binding.selection.root),&["show","-s","--format=%B",base])?!=format!("Nola task source snapshot {}",snapshot.plan.state_sha256) {
        return Err("Frozen source snapshot object differs from its binding".into());
    }
    if fresh_source {
        let state=source_state(root,None,None)?;
        if state.plan!=snapshot.plan || state.untracked || worktree_repository(root)?.0!=Path::new(&binding.selection.expected_common_directory) {
            return Err("Source changed since the frozen snapshot; inspect before integration".into());
        }
        let changed=git_command_output(root,&["diff","--name-only","--no-renames","--no-ext-diff","--no-textconv","-z",&snapshot.plan.source_head,base,"--","."])?;
        for path in changed.split(|byte|*byte==0).filter(|path|!path.is_empty()) {
            let path=std::str::from_utf8(path).map_err(|_|"Invalid snapshot tree path")?;
            if !state.entries.iter().any(|entry|entry.path==path) {return Err("Snapshot changes a path absent from its source witness".into());}
        }
        for entry in &state.entries {
            let listed=git_command_output(root,&["--literal-pathspecs","ls-tree","-z",base,"--",&entry.path])?;
            if entry.mode==0 {
                if !listed.is_empty() {return Err("Snapshot did not preserve a source deletion".into());}
                continue;
            }
            let record=std::str::from_utf8(&listed).map_err(|_|"Invalid snapshot blob entry")?;
            let header=record.split('\t').next().ok_or("Missing snapshot tree header")?.split_whitespace().collect::<Vec<_>>();
            if header.len()!=3 || header[0]!=format!("{:o}",entry.mode) || header[1]!="blob" {return Err("Snapshot file mode differs from source".into());}
            let bytes=git_command_output(root,&["cat-file","blob",header[2]])?;
            if bytes.len()>8*1024*1024 || entry.sha256.as_deref()!=Some(format!("{:x}",<sha2::Sha256 as sha2::Digest>::digest(&bytes)).as_str()) {
                return Err("Snapshot file bytes differ from source".into());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod git_worktree_source_snapshot_tests {
    use super::*;
    #[test]
    fn tracked_dirty_snapshot_preserves_index_and_integrates_only_against_exact_source() {
        for bare in [false,true] {
            let fixture=tempfile::tempdir().unwrap();let parent=fixture.path().canonicalize().unwrap();
            let seed=parent.join("seed");fs::create_dir(&seed).unwrap();
            for args in [vec!["init","--quiet"],vec!["config","user.name","Fixture"],vec!["config","user.email","fixture@localhost"],
                vec!["config","commit.gpgsign","false"],vec!["config","core.hooksPath",".git/no-hooks"]] {git_command_output(&seed,&args).unwrap();}
            fs::write(seed.join("code.txt"),"base\n").unwrap();fs::write(seed.join("gone.txt"),"old\n").unwrap();
            git_command_output(&seed,&["add","."]).unwrap();git_command_output(&seed,&["commit","--quiet","-m","base"]).unwrap();
            let root=if bare {
                let common=parent.join("repo.git");
                git_command_output(&seed,&["clone","--quiet","--bare","--local",&git_path_argument(&seed).unwrap(),&git_path_argument(&common).unwrap()]).unwrap();
                for args in [vec!["config","user.name","Fixture"],vec!["config","user.email","fixture@localhost"],vec!["config","commit.gpgsign","false"]] {git_command_output(&common,&args).unwrap();}
                let source=parent.join("source");git_command_output(&common,&["worktree","add","--quiet","--detach",&git_path_argument(&source).unwrap(),"HEAD"]).unwrap();source
            } else {seed};
            fs::write(root.join("code.txt"),"user staged\n").unwrap();git_command_output(&root,&["add","code.txt"]).unwrap();
            fs::write(root.join("code.txt"),"user working\n").unwrap();fs::remove_file(root.join("gone.txt")).unwrap();
            fs::write(root.join("new-code.rs"),"new tracked code\n").unwrap();git_command_output(&root,&["add","new-code.rs"]).unwrap();
            let ctx=ToolContext {memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),allowed_dirs:vec![parent.clone()],language_services:Default::default(),
                skills:crate::skill_runtime::SkillRuntimeContext::from_configured_dirs(None),
                policy:ToolExecutionPolicy::new(Vec::new(),false,"ask".into(),Vec::new()),permission_scope:Default::default(),
                cancellation:Default::default(),plan_state:Default::default(),output_store:ToolOutputStore::for_test(),browser:BrowserRuntime::new(parent.join("browser"))};
            let before_index=git_command_output(&root,&["ls-files","--stage","-z"]).unwrap();
            let info=inspect_git_worktrees(&root).unwrap();assert!(info["source_snapshot_plan"].is_object());
            let child=parent.join("child");
            let input=json!({"root":root,"destination":child,"expected_head":info["head"],"expected_common_directory":info["common_directory"],"source_snapshot":info["source_snapshot_plan"]});
            fs::write(root.join("private-untracked.txt"),"must not be copied").unwrap();
            assert!(inspect_git_worktrees(&root).unwrap()["source_snapshot_plan"].is_null());
            assert!(git_worktree_create(input.as_object().unwrap(),&ctx).error);assert!(!child.exists());
            fs::remove_file(root.join("private-untracked.txt")).unwrap(); // Only this owned synthetic marker.
            let made=git_worktree_create(input.as_object().unwrap(),&ctx);assert!(!made.error,"{}",made.text);
            let created=made.data.unwrap();let selection=created["task_worktree_selection"].clone();
            assert_ne!(selection["expected_head"],info["head"]);
            assert_eq!(fs::read_to_string(child.join("code.txt")).unwrap(),"user working\n");
            assert_eq!(fs::read_to_string(child.join("new-code.rs")).unwrap(),"new tracked code\n");assert!(!child.join("gone.txt").exists());
            assert_eq!(git_command_output(&root,&["ls-files","--stage","-z"]).unwrap(),before_index);
            assert_eq!(git_commit_text(&root,&["rev-parse","HEAD"]).unwrap(),info["head"]);
            let selected=serde_json::from_value(selection.clone()).unwrap();let binding=capture_task_worktree(&selected,&ctx.allowed_dirs).unwrap();
            fs::write(child.join("code.txt"),"user working\nagent delivery\n").unwrap();
            git_command_output(&child,&["add","."]).unwrap();git_command_output(&child,&["commit","--quiet","-m","task delivery"]).unwrap();
            let integrate=json!({"root":root,"worktree":selection,"expected_child_head":git_commit_text(&child,&["rev-parse","HEAD"]).unwrap()});
            fs::write(root.join("code.txt"),"concurrent user edit\n").unwrap();
            verify_task_worktree(&binding).unwrap(); // Cold worker may keep using its immutable snapshot.
            let primary=nola_domain::task_workspace::PrimaryWorktreeBinding {source_root:root.to_string_lossy().into_owned(),
                source_repository:observe_task_repository(&root).unwrap(),selection:selected.clone()};
            assert_eq!(primary_execution_root(&primary,&root,&[parent.clone()]).unwrap(),child);
            assert!(primary_execution_root(&primary,&root,&[root.clone()]).is_err());
            assert!(primary_execution_root(&primary,&child,&[parent.clone()]).is_err());
            assert!(git_worktree_integrate(integrate.as_object().unwrap(),&ctx).error);
            assert_eq!(fs::read_to_string(root.join("code.txt")).unwrap(),"concurrent user edit\n");
            fs::write(root.join("code.txt"),"user working\n").unwrap(); // Restore only the owned fixture baseline.
            let applied=git_worktree_integrate(integrate.as_object().unwrap(),&ctx);assert!(!applied.error,"{}",applied.text);
            assert_eq!(fs::read_to_string(root.join("code.txt")).unwrap(),"user working\nagent delivery\n");
            assert_eq!(git_command_output(&root,&["ls-files","--stage","-z"]).unwrap(),before_index);
            assert_eq!(git_commit_text(&root,&["rev-parse","HEAD"]).unwrap(),info["head"]);
            assert!(child.exists());assert!(git_worktree_integrate(integrate.as_object().unwrap(),&ctx).error);
        }
    }
}
