use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const SKIP_DIRS: &[&str] = &["target", ".git", "dist", "node_modules", "stage"];
const AI_FILES: &[&str] = &[
    "CLAUDE.md",
    "AGENTS.md",
    "GEMINI.md",
    ".claude",
    ".cursor",
    ".cursorrules",
    ".aider.conf.yml",
    ".windsurfrules",
    "copilot-instructions.md",
];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            if !SKIP_DIRS.contains(&name.as_str()) {
                walk(&path, out);
            }
        } else {
            out.push(path);
        }
    }
}

fn files() -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk(&root(), &mut out);
    out.sort();
    out
}

fn rel(p: &Path) -> String {
    p.strip_prefix(root()).unwrap_or(p).display().to_string()
}

fn ext(p: &Path) -> String {
    p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

fn rust_comment(line: &str, in_block: &mut bool, in_string: &mut bool) -> bool {
    let b = line.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if *in_block {
            return true;
        }
        let c = b[i];
        if *in_string {
            if c == b'\\' {
                i += 2;
                continue;
            }
            if c == b'"' {
                *in_string = false;
            }
            i += 1;
            continue;
        }
        match c {
            b'"' => *in_string = true,
            b'\'' => {
                if i + 2 < b.len() && b[i + 2] == b'\'' {
                    i += 3;
                    continue;
                }
                if i + 3 < b.len() && b[i + 1] == b'\\' && b[i + 3] == b'\'' {
                    i += 4;
                    continue;
                }
            }
            b'/' if i + 1 < b.len() && (b[i + 1] == b'/' || b[i + 1] == b'*') => return true,
            _ => {}
        }
        i += 1;
    }
    false
}

fn nocomments() -> bool {
    let mut bad = Vec::new();
    for f in files() {
        let Ok(text) = std::fs::read_to_string(&f) else { continue };
        match ext(&f).as_str() {
            "rs" => {
                let mut block = false;
                let mut string = false;
                for (n, line) in text.lines().enumerate() {
                    if rust_comment(line, &mut block, &mut string) {
                        bad.push(format!("{}:{}", rel(&f), n + 1));
                    }
                }
            }
            "toml" | "yml" | "yaml" => {
                for (n, line) in text.lines().enumerate() {
                    if line.trim_start().starts_with('#') {
                        bad.push(format!("{}:{}", rel(&f), n + 1));
                    }
                }
            }
            "iss" => {
                for (n, line) in text.lines().enumerate() {
                    if line.trim_start().starts_with(';') {
                        bad.push(format!("{}:{}", rel(&f), n + 1));
                    }
                }
            }
            _ => {}
        }
    }
    report("comments", bad)
}

fn is_text(p: &Path) -> bool {
    matches!(
        ext(p).as_str(),
        "rs" | "toml" | "yml" | "yaml" | "md" | "iss" | "desktop" | "svg" | "json" | "txt" | "ps1" | "sh"
    )
}

fn names() -> bool {
    let mut bad = Vec::new();
    for f in files() {
        if !is_text(&f) || rel(&f).starts_with("THIRD-PARTY") || rel(&f) == "xtask/src/main.rs" {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&f) else { continue };
        for (n, line) in text.lines().enumerate() {
            let lower = line.to_lowercase();
            let home = ["/home/", "/users/", "c:\\users\\", "/run/media/", "/media/"].iter().any(|p| lower.contains(p));
            let email = line.split(|c: char| c.is_whitespace() || "<>()\"'".contains(c)).any(|w| {
                let w = w.trim_matches(|c: char| !c.is_alphanumeric());
                w.contains('@')
                    && w.split('@').nth(1).is_some_and(|d| d.contains('.'))
                    && !w.ends_with("noreply.github.com")
                    && !w.contains("${{")
            });
            if home || email {
                bad.push(format!("{}:{}", rel(&f), n + 1));
            }
        }
    }
    report("personal paths or email addresses", bad)
}

fn aifiles() -> bool {
    let mut bad = Vec::new();
    let mut all = files();
    let mut dirs = Vec::new();
    collect_dirs(&root(), &mut dirs);
    all.extend(dirs);
    for f in all {
        let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if AI_FILES.contains(&name.as_str()) {
            bad.push(rel(&f));
        }
    }
    report("assistant files", bad)
}

fn collect_dirs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() && !SKIP_DIRS.contains(&name.as_str()) {
            out.push(path.clone());
            collect_dirs(&path, out);
        }
    }
}

fn report(what: &str, bad: Vec<String>) -> bool {
    if bad.is_empty() {
        println!("no {what} found");
        return true;
    }
    eprintln!("found {what}:");
    for b in &bad {
        eprintln!("  {b}");
    }
    false
}

fn license_text(dir: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return out };
    let mut names: Vec<_> = entries.flatten().map(|e| e.path()).collect();
    names.sort();
    for p in names {
        let n = p.file_name().map(|n| n.to_string_lossy().to_uppercase()).unwrap_or_default();
        if (n.starts_with("LICENSE") || n.starts_with("LICENCE") || n.starts_with("COPYING") || n.starts_with("NOTICE"))
            && p.is_file()
        {
            if let Ok(t) = std::fs::read_to_string(&p) {
                out.push((p.file_name().unwrap().to_string_lossy().into_owned(), t));
            }
        }
    }
    out
}

fn licenses(check: bool) -> bool {
    let output = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args(["metadata", "--format-version", "1", "--locked"])
        .current_dir(root())
        .output();
    let Some(output) = output.ok().filter(|o| o.status.success()) else {
        eprintln!("cargo metadata failed");
        return false;
    };
    let meta: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap_or_default();
    let ours = ["tgow-common", "tgow-rec", "tgow-daemon", "tgow-overlay", "xtask"];
    let mut crates: BTreeMap<String, serde_json::Value> = BTreeMap::new();
    for p in meta["packages"].as_array().into_iter().flatten() {
        let name = p["name"].as_str().unwrap_or_default();
        if ours.contains(&name) {
            continue;
        }
        crates.insert(format!("{} {}", name, p["version"].as_str().unwrap_or_default()), p.clone());
    }
    let mut text = String::new();
    text.push_str("TGOW Recorder uses the following open-source software.\n\n");
    text.push_str("FFmpeg\n  License: LGPL-2.1-or-later\n  Source: https://ffmpeg.org\n");
    text.push_str("  The Windows builds ship the shared FFmpeg libraries unmodified, built by https://github.com/BtbN/FFmpeg-Builds.\n");
    text.push_str("  The Linux builds use the FFmpeg libraries of your system.\n\n");
    let mut seen = std::collections::HashSet::new();
    for (id, p) in &crates {
        text.push_str(&"=".repeat(78));
        text.push('\n');
        text.push_str(id);
        text.push('\n');
        text.push_str(&format!("  License: {}\n", p["license"].as_str().unwrap_or("see below")));
        if let Some(repo) = p["repository"].as_str() {
            text.push_str(&format!("  Source: {repo}\n"));
        }
        let dir = Path::new(p["manifest_path"].as_str().unwrap_or_default())
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        for (file, body) in license_text(&dir) {
            let key = body.split_whitespace().collect::<Vec<_>>().join(" ");
            if seen.insert(key) {
                text.push_str(&format!("\n  {file}\n\n"));
                text.push_str(&body);
                if !body.ends_with('\n') {
                    text.push('\n');
                }
            } else {
                text.push_str(&format!("  {file}: same text as printed above\n"));
            }
        }
        text.push('\n');
    }
    let path = root().join("THIRD-PARTY-LICENSES.txt");
    if check {
        let current = std::fs::read_to_string(&path).unwrap_or_default();
        if current != text {
            eprintln!("THIRD-PARTY-LICENSES.txt is out of date, run cargo xtask licenses");
            return false;
        }
        println!("THIRD-PARTY-LICENSES.txt is up to date");
        return true;
    }
    if std::fs::write(&path, text).is_err() {
        eprintln!("could not write {}", path.display());
        return false;
    }
    println!("wrote THIRD-PARTY-LICENSES.txt ({} crates)", crates.len());
    true
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let ok = match args.first().map(String::as_str) {
        Some("nocomments") => nocomments(),
        Some("names") => names(),
        Some("aifiles") => aifiles(),
        Some("licenses") => licenses(args.iter().any(|a| a == "--check")),
        Some("check") => [nocomments(), names(), aifiles()].iter().all(|v| *v),
        _ => {
            eprintln!("usage: cargo xtask <nocomments|names|aifiles|licenses [--check]|check>");
            false
        }
    };
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
