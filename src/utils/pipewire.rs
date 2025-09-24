#[cfg(target_os = "linux")]
use std::process::Command;

#[cfg(target_os = "linux")]
#[derive(Debug, Clone)]
pub struct PwSource {
    pub id: u32,
    pub name: String,
    #[allow(dead_code)]
    pub raw: String,
}

#[cfg(target_os = "linux")]
pub fn list_pw_sources() -> Vec<PwSource> {
    // Try wpctl status first
    if let Ok(out) = Command::new("wpctl").arg("status").output() {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout);
            eprintln!("[PW] wpctl status stdout:\n{}", s);
            return parse_wpctl_status_sources(&s);
        }
    }
    // Fallback to pactl list short sources
    if let Ok(out) = Command::new("pactl")
        .args(["list", "short", "sources"])
        .output()
    {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout);
            eprintln!("[PW] pactl list short sources stdout:\n{}", s);
            return parse_pactl_short_sources(&s);
        }
    }
    Vec::new()
}

#[cfg(target_os = "linux")]
fn parse_wpctl_status_sources(text: &str) -> Vec<PwSource> {
    let mut res = Vec::new();
    let mut in_audio = false;
    let mut in_sources = false;
    for line in text.lines() {
        let l = line.trim_end();
        if l.starts_with("Audio") {
            in_audio = true;
            continue;
        }
        if in_audio && l.starts_with("Video") {
            break;
        }
        if in_audio && l.contains("Sources:") {
            in_sources = true;
            continue;
        }
        if in_audio
            && (l.starts_with("Sinks:") || l.starts_with("Filters:") || l.starts_with("Devices:"))
        {
            in_sources = false;
        }
        if in_audio && in_sources {
            // Format examples: "│      69. Anker PowerConf C200 Analog Stereo [vol: 1.50]"
            let trimmed = l.trim_start_matches('│').trim();
            if let Some(dotpos) = trimmed.find('.') {
                let (id_part, rest) = trimmed.split_at(dotpos);
                let id_str = id_part.trim();
                let rest = rest.trim_start_matches('.').trim();
                if let Ok(id) = id_str.parse::<u32>() {
                    // name until end or before bracket
                    let name = rest.split('[').next().unwrap_or(rest).trim().to_string();
                    res.push(PwSource {
                        id,
                        name: name.clone(),
                        raw: trimmed.to_string(),
                    });
                }
            }
        }
    }
    eprintln!("[PW] parsed wpctl sources: {:?}", res);
    res
}

#[cfg(target_os = "linux")]
fn parse_pactl_short_sources(text: &str) -> Vec<PwSource> {
    let mut res = Vec::new();
    for line in text.lines() {
        // Format: ID\tNAME\tSERVER\tFORMAT\tSTATE
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() >= 2 {
            if let Ok(id) = cols[0].parse::<u32>() {
                let name = cols[1].to_string();
                res.push(PwSource {
                    id,
                    name: name.clone(),
                    raw: line.to_string(),
                });
            }
        }
    }
    eprintln!("[PW] parsed pactl sources: {:?}", res);
    res
}

#[cfg(not(target_os = "linux"))]
#[derive(Debug, Clone)]
pub struct PwSource {
    pub id: u32,
    pub name: String,
    #[allow(dead_code)]
    pub raw: String,
}

#[cfg(not(target_os = "linux"))]
pub fn list_pw_sources() -> Vec<PwSource> {
    Vec::new()
}
