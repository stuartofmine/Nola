use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let source_root = manifest_dir
        .parent()
        .unwrap()
        .join("NolaMac/Resources/NolaBuiltInSkills.bundle");
    println!("cargo:rerun-if-changed={}", source_root.display());

    let mut skill_dirs = fs::read_dir(&source_root)
        .expect("read shared built-in Skill bundle")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir() && path.join("SKILL.md").is_file())
        .collect::<Vec<_>>();
    skill_dirs.sort();

    let mut generated = String::from("&[\n");
    for directory in skill_dirs {
        let instruction_path = directory.join("SKILL.md");
        let instruction = fs::read_to_string(&instruction_path).expect("read built-in SKILL.md");
        let resource_dir = directory.file_name().unwrap().to_string_lossy().to_string();
        let name = frontmatter_value(&instruction, "name").unwrap_or_else(|| resource_dir.clone());
        let description = frontmatter_value(&instruction, "description").unwrap_or_default();
        let title = instruction
            .lines()
            .find_map(|line| line.strip_prefix("# "))
            .unwrap_or(&name)
            .trim()
            .to_string();
        let mut files = Vec::new();
        collect_files(&directory, &directory, &mut files);
        files.sort();

        generated.push_str("    EmbeddedSkill {\n");
        generated.push_str(&format!("        id: {:?},\n", name));
        generated.push_str(&format!("        resource_dir: {:?},\n", resource_dir));
        generated.push_str(&format!("        title: {:?},\n", title));
        generated.push_str(&format!("        description: {:?},\n", description));
        generated.push_str(&format!(
            "        instruction: include_str!({:?}),\n",
            instruction_path
        ));
        generated.push_str(&format!(
            "        user_facing: {},\n",
            resource_dir != "nature-shared"
        ));
        generated.push_str("        files: &[\n");
        for path in files {
            let relative = path.strip_prefix(&directory).unwrap();
            if relative == Path::new("SKILL.md") {
                continue;
            }
            generated.push_str(&format!(
                "            EmbeddedSkillFile {{ relative_path: {:?}, contents: include_bytes!({:?}) }},\n",
                relative.to_string_lossy(),
                path
            ));
        }
        generated.push_str("        ],\n    },\n");
    }
    generated.push_str("]\n");

    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("builtin_skills_generated.rs");
    fs::write(out, generated).expect("write generated built-in Skill catalog");
}

fn frontmatter_value(markdown: &str, key: &str) -> Option<String> {
    let mut lines = markdown.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    for line in lines {
        if line.trim() == "---" {
            break;
        }
        if let Some(value) = line.strip_prefix(&format!("{key}:")) {
            return Some(value.trim().trim_matches('"').to_string());
        }
    }
    None
}

fn collect_files(root: &Path, directory: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|name| name == "__pycache__") {
                continue;
            }
            collect_files(root, &path, out);
        } else if path.extension().is_none_or(|extension| extension != "pyc")
            && path.starts_with(root)
        {
            out.push(path);
        }
    }
}
