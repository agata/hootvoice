use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

use crate::utils::app_config_dir;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DictionaryEntry {
    pub canonical: String,
    pub aliases: Vec<String>,
    /// Apply only when the full input text contains any of these terms.
    /// If omitted or empty, the entry always applies.
    #[serde(default)]
    pub include: Vec<String>,
}

pub type Dictionary = Vec<DictionaryEntry>;

/// The filename under the app config dir
pub const DICTIONARY_FILENAME: &str = "dictionary.yaml";

/// Return the absolute path to the dictionary YAML in the app's config dir.
pub fn dictionary_path() -> PathBuf {
    app_config_dir().join(DICTIONARY_FILENAME)
}

/// Load dictionary from YAML file. If the file doesn't exist, create it with a sample.
pub fn load_or_init_dictionary() -> anyhow::Result<Dictionary> {
    let path = dictionary_path();
    if !path.exists() {
        // Ensure parent exists
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let sample = default_sample_yaml();
        fs::write(&path, sample)?;
    }
    let s = fs::read_to_string(&path)?;
    let dict: Dictionary = serde_yaml::from_str(&s).unwrap_or_else(|_| Vec::new());
    Ok(dict)
}

/// Save dictionary to YAML format.
pub fn save_dictionary(dict: &Dictionary) -> anyhow::Result<()> {
    let path = dictionary_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let yaml = serde_yaml::to_string(dict)?;
    fs::write(&path, yaml)?;
    Ok(())
}

// Build flattened (alias -> canonical) pairs sorted by alias length descending.
// removed: flatten_sorted (unused)
// Note: kept as a normal comment to avoid doc-confusion lint
/// Filter by `include` terms and return alias -> canonical pairs sorted by longest alias first
pub fn flatten_sorted_with_context(dict: &Dictionary, context_text: &str) -> Vec<(String, String)> {
    let ctx_lower = context_text.to_lowercase();
    let mut pairs: Vec<(String, String)> = Vec::new();
    for entry in dict.iter() {
        // If `include` is empty apply unconditionally; otherwise require any term to match
        let applicable = if entry.include.is_empty() {
            true
        } else {
            entry
                .include
                .iter()
                .any(|k| !k.is_empty() && ctx_lower.contains(&k.to_lowercase()))
        };
        if !applicable {
            continue;
        }
        for a in entry.aliases.iter() {
            if !a.is_empty() {
                pairs.push((a.clone(), entry.canonical.clone()));
            }
        }
    }
    pairs.sort_by(|(a1, _), (a2, _)| a2.len().cmp(&a1.len()));
    pairs
}

/// Apply dictionary pairs to text using longest-first replacement.
pub fn apply_pairs(text: &str, pairs: &[(String, String)]) -> String {
    if pairs.is_empty() || text.is_empty() {
        return text.to_string();
    }

    // Collect non-overlapping replacement ranges from the original text so that
    // later pairs don't match text produced by earlier replacements.
    let mut ranges: Vec<(usize, usize, &str)> = Vec::new();
    for (alias, canon) in pairs.iter() {
        if alias.is_empty() {
            continue;
        }
        for (start, _) in text.match_indices(alias) {
            let end = start + alias.len();
            // Skip if this match overlaps an existing replacement.
            if ranges.iter().any(|&(s, e, _)| start < e && s < end) {
                continue;
            }
            ranges.push((start, end, canon));
        }
    }
    if ranges.is_empty() {
        return text.to_string();
    }

    ranges.sort_by_key(|r| r.0);
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for (start, end, canon) in ranges {
        out.push_str(&text[last..start]);
        out.push_str(canon);
        last = end;
    }
    out.push_str(&text[last..]);
    out
}

fn default_sample_yaml() -> String {
    // Provide a simple readable YAML sample with common tech words
    // Users can edit via the Settings > Dictionary tab.
    let sample = r#"
- canonical: "TypeScript"
  aliases: ["Type Script", "TS", "TS language"]
- canonical: "Whisper"
  aliases: ["whisper.cpp", "whisper-rs"]
- canonical: "Rust"
  aliases: ["Rustlang", "Rust language"]
  include: ["language", "build", "compile", "dev", "cargo", "crate"]
"#;
    sample.to_string()
}

#[cfg(test)]
mod tests {
    use super::{apply_pairs, flatten_sorted_with_context, DictionaryEntry};

    #[test]
    fn no_recursive_replacement() {
        let dict = vec![
            DictionaryEntry {
                canonical: "1".into(),
                aliases: vec!["12".into()],
                include: vec![],
            },
            DictionaryEntry {
                canonical: "one".into(),
                aliases: vec!["1".into()],
                include: vec![],
            },
        ];
        let pairs = flatten_sorted_with_context(&dict, "");
        assert_eq!(apply_pairs("12 1", &pairs), "1 one");
    }

    #[test]
    fn longest_first_without_overlap() {
        let pairs = vec![("foobar".into(), "X".into()), ("foo".into(), "Y".into())];
        assert_eq!(apply_pairs("foobar foo", &pairs), "X Y");
    }
}
