use fluent_bundle::{FluentBundle, FluentResource};
use once_cell::sync::Lazy;
use std::borrow::Cow;
use std::sync::RwLock;
use unic_langid::LanguageIdentifier;

// Normalize locale strings like "ja_JP.UTF-8" or "ja-JP" to BCP47-ish form
// Example: "ja_JP.UTF-8" -> "ja-JP"
fn normalize_locale_tag<S: AsRef<str>>(s: S) -> String {
    let mut tag = s.as_ref().trim().to_string();
    if let Some((lang_region, _encoding)) = tag.split_once('.') {
        tag = lang_region.to_string();
    }
    tag = tag.replace('_', "-");
    tag
}

fn detect_lang() -> LanguageIdentifier {
    // 1) Explicit override via env var
    if let Ok(s) = std::env::var("HOOTVOICE_UI_LANG") {
        let s = s.trim();
        if !s.is_empty() && s != "auto" {
            let norm = normalize_locale_tag(s);
            if let Ok(li) = norm.parse::<LanguageIdentifier>() {
                return li;
            }
        }
    }

    // 2) OS/UI locale via sys-locale (cross-platform; uses Windows API on Windows)
    if let Some(loc) = sys_locale::get_locale() {
        let norm = normalize_locale_tag(&loc);
        if let Ok(li) = norm.parse::<LanguageIdentifier>() {
            return li;
        }
        // Heuristic fallback by language prefix
        let low = norm.to_lowercase();
        if low.starts_with("ja") {
            return "ja".parse().unwrap();
        }
        if low.starts_with("en") {
            return "en-US".parse().unwrap();
        }
    }

    // 3) Common UNIX envs as a last resort
    for key in ["LC_ALL", "LC_MESSAGES", "LANG"].iter() {
        if let Ok(val) = std::env::var(key) {
            let norm = normalize_locale_tag(&val);
            if let Ok(li) = norm.parse::<LanguageIdentifier>() {
                return li;
            }
            let low = norm.to_lowercase();
            if low.starts_with("ja") {
                return "ja".parse().unwrap();
            }
            if low.starts_with("en") {
                return "en-US".parse().unwrap();
            }
        }
    }

    // 4) Default: English
    "en-US".parse().unwrap()
}

fn build_bundle(pref: Option<&str>) -> FluentBundle<FluentResource> {
    let lang = match pref.map(|s| s.trim().to_lowercase()) {
        Some(ref p) if p == "ja" => "ja".parse().unwrap(),
        Some(ref p) if p == "en" || p == "en-us" => "en-US".parse().unwrap(),
        _ => detect_lang(),
    };
    let mut bundle = FluentBundle::new(vec![lang.clone()]);
    let ftl: &str = match lang.language.as_str() {
        "ja" => include_str!("../i18n/ja/app.ftl"),
        _ => include_str!("../i18n/en/app.ftl"),
    };
    let resource = match FluentResource::try_new(ftl.to_owned()) {
        Ok(res) => res,
        Err(e) => {
            eprintln!(
                "Warning: failed to parse FTL for {:?}: {:?}. Falling back to English.",
                lang, e
            );
            if lang.language != "en" {
                return build_bundle(Some("en-US"));
            } else {
                return FluentBundle::new(vec![lang]);
            }
        }
    };

    if let Err(e) = bundle.add_resource(resource) {
        eprintln!(
            "Warning: failed to add FTL resource for {:?}: {:?}. Falling back to English.",
            lang, e
        );
        if lang.language != "en" {
            return build_bundle(Some("en-US"));
        }
    }

    bundle
}

static LANG_PREF: Lazy<RwLock<String>> = Lazy::new(|| RwLock::new(String::from("auto")));

// Store UI language preference (auto/ja/en); rebuild bundle on each call
pub fn set_ui_language_preference(pref: &str) {
    let mut g = LANG_PREF.write().expect("i18n pref lock poisoned");
    *g = pref.to_string();
}

pub fn tr(id: &str) -> String {
    let pref = {
        let g = LANG_PREF.read().expect("i18n pref lock poisoned");
        g.clone()
    };
    let bundle = build_bundle(Some(&pref));
    if let Some(msg) = bundle.get_message(id) {
        if let Some(pattern) = msg.value() {
            let mut errors = vec![];
            let value: Cow<str> = bundle.format_pattern(pattern, None, &mut errors);
            return value.into_owned();
        }
    }
    id.to_string()
}
