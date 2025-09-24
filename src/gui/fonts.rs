use eframe::egui;
use std::sync::Arc;

fn read_first_existing(paths: &[&str]) -> Option<Vec<u8>> {
    for p in paths {
        if let Ok(data) = std::fs::read(p) {
            return Some(data);
        }
    }
    None
}

pub fn setup_custom_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    // Candidate paths for CJK fonts
    #[cfg(target_os = "macos")]
    let cjk_candidates = [
        // macOS system fonts
        "/System/Library/Fonts/Hiragino Sans GB.ttc",
        "/Library/Fonts/Hiragino Sans GB W3.ttc",
        "/Library/Fonts/Hiragino Sans GB W6.ttc",
        "/System/Library/Fonts/PingFang.ttc",
        "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
        // Noto CJK (if installed via Homebrew)
        "/opt/homebrew/share/fonts/NotoSansCJK-Regular.ttc",
        "/usr/local/share/fonts/NotoSansCJK-Regular.ttc",
        // Other fonts
        "/Library/Fonts/Yu Gothic Medium.otf",
        "/Library/Fonts/YuGothic-Medium.otf",
    ];

    // Windows system font locations
    #[cfg(target_os = "windows")]
    let cjk_candidates = [
        // Yu Gothic / Yu Gothic UI
        "C:\\Windows\\Fonts\\YuGothR.ttc",
        "C:\\Windows\\Fonts\\YuGothM.ttc",
        "C:\\Windows\\Fonts\\YuGothL.ttc",
        "C:\\Windows\\Fonts\\YuGothB.ttc",
        "C:\\Windows\\Fonts\\YuGothUIR.ttc",
        "C:\\Windows\\Fonts\\YuGothUIM.ttc",
        "C:\\Windows\\Fonts\\YuGothUIL.ttc",
        // Meiryo (widely available)
        "C:\\Windows\\Fonts\\meiryo.ttc",
        "C:\\Windows\\Fonts\\Meiryo.ttc",
        // MS Gothic (older but very common)
        "C:\\Windows\\Fonts\\msgothic.ttc",
        "C:\\Windows\\Fonts\\MSMINCHO.TTC",
    ];

    // Linux and other Unix-like systems
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    let cjk_candidates = [
        // Noto CJK
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/noto/NotoSansCJKjp-Regular.otf",
        // IPA fonts (fallbacks)
        "/usr/share/fonts/opentype/ipafont-gothic/ipag.ttf",
        "/usr/share/fonts/truetype/fonts-japanese-gothic.ttf",
        // DejaVu (Latin fallback)
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    ];

    // Candidate paths for emoji fonts (prefer monochrome; color as fallback)
    #[cfg(target_os = "macos")]
    let emoji_bw_candidates = [
        // Bundled assets first
        "assets/fonts/NotoEmoji-Regular.ttf",
        "assets/fonts/TwemojiMozilla.ttf",
        "assets/fonts/OpenMoji-Regular.ttf",
        "assets/fonts/Symbola.ttf",
        // macOS emoji fonts
        "/System/Library/Fonts/Apple Color Emoji.ttc",
        "/System/Library/Fonts/Supplemental/Apple Symbols.ttf",
    ];

    #[cfg(target_os = "windows")]
    let emoji_bw_candidates = [
        // Bundled assets first (if present)
        "assets/fonts/NotoEmoji-Regular.ttf",
        "assets/fonts/TwemojiMozilla.ttf",
        "assets/fonts/OpenMoji-Regular.ttf",
        "assets/fonts/Symbola.ttf",
        // Windows symbol fallback (monochrome)
        "C:\\Windows\\Fonts\\seguisym.ttf",
    ];

    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    let emoji_bw_candidates = [
        // Bundled assets first
        "assets/fonts/NotoEmoji-Regular.ttf",
        "assets/fonts/TwemojiMozilla.ttf",
        "assets/fonts/OpenMoji-Regular.ttf",
        "assets/fonts/Symbola.ttf",
        // System locations
        "/usr/share/fonts/truetype/noto/NotoEmoji-Regular.ttf",
        "/usr/share/fonts/truetype/twemoji/TwemojiMozilla.ttf",
        "/usr/share/fonts/joypixels/JoyPixels.ttf",
        "/usr/share/fonts/truetype/ancient-scripts/Symbola.ttf",
    ];

    #[cfg(target_os = "macos")]
    let emoji_color_candidates = [
        // Bundled asset
        "assets/fonts/NotoColorEmoji.ttf",
        // macOS color emoji
        "/System/Library/Fonts/Apple Color Emoji.ttc",
    ];

    #[cfg(target_os = "windows")]
    let emoji_color_candidates = [
        // Bundled asset
        "assets/fonts/NotoColorEmoji.ttf",
        // Windows color emoji
        "C:\\Windows\\Fonts\\seguiemj.ttf",
    ];

    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    let emoji_color_candidates = [
        // Bundled asset
        "assets/fonts/NotoColorEmoji.ttf",
        // System locations
        "/usr/share/fonts/truetype/noto/NotoColorEmoji.ttf",
        "/usr/share/fonts/emoji/NotoColorEmoji.ttf",
        "/usr/share/fonts/noto/NotoColorEmoji.ttf",
        "/usr/share/fonts/NotoColorEmoji.ttf",
    ];

    // Add Lucide icon font first for reliable icon rendering
    fonts.font_data.insert(
        "lucide".to_owned(),
        Arc::new(egui::FontData::from_static(
            lucide_icons::lucide_font_bytes(),
        )),
    );
    // Prepend Lucide to proportional families so fallback picks it up
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(0, "lucide".to_owned());
    // Bind for explicit family name (FontFamily::Name("lucide"))
    fonts
        .families
        .entry(egui::FontFamily::Name("lucide".into()))
        .or_default()
        .insert(0, "lucide".to_owned());

    // Prefer monochrome first (more compatible)
    if let Some(emoji_bw) = read_first_existing(&emoji_bw_candidates) {
        fonts.font_data.insert(
            "emoji_bw".to_owned(),
            Arc::new(egui::FontData::from_owned(emoji_bw)),
        );
        for fam in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            fonts
                .families
                .entry(fam)
                .or_default()
                .insert(0, "emoji_bw".to_owned());
        }
    } else if let Some(emoji_color) = read_first_existing(&emoji_color_candidates) {
        // Color emoji may not render with color in egui; add with lower priority
        fonts.font_data.insert(
            "emoji_color".to_owned(),
            Arc::new(egui::FontData::from_owned(emoji_color)),
        );
        for fam in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            fonts
                .families
                .entry(fam)
                .or_default()
                .push("emoji_color".to_owned());
        }
    }

    if let Some(cjk) = read_first_existing(&cjk_candidates) {
        fonts.font_data.insert(
            "cjk_fallback".to_owned(),
            Arc::new(egui::FontData::from_owned(cjk)),
        );
        // Prefer CJK glyphs via fallback
        fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .insert(0, "cjk_fallback".to_owned());
        fonts
            .families
            .entry(egui::FontFamily::Monospace)
            .or_default()
            .insert(0, "cjk_fallback".to_owned());
    }

    ctx.set_fonts(fonts);
}
