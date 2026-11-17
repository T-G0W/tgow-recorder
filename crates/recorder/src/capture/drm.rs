#[derive(Debug, Clone)]
pub struct Connector {
    pub name: String,
    pub width: u32,
    pub height: u32,
}

pub fn connected() -> Vec<Connector> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir("/sys/class/drm") else {
        return out;
    };
    let mut names: Vec<_> = entries.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    for entry in names {
        let Some((_, connector)) = entry.split_once('-') else { continue };
        let base = std::path::Path::new("/sys/class/drm").join(&entry);
        let status = std::fs::read_to_string(base.join("status")).unwrap_or_default();
        if status.trim() != "connected" {
            continue;
        }
        let modes = std::fs::read_to_string(base.join("modes")).unwrap_or_default();
        let (width, height) = modes
            .lines()
            .next()
            .and_then(|m| m.split_once('x'))
            .and_then(|(w, h)| Some((w.parse().ok()?, h.trim_end_matches(|c: char| !c.is_ascii_digit()).parse().ok()?)))
            .unwrap_or((0, 0));
        out.push(Connector { name: connector.to_string(), width, height });
    }
    out
}
