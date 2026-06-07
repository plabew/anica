use std::fs;

use cosmic_text::FontSystem;

pub(crate) fn load_extra_fonts(font_system: &mut FontSystem) {
    let Some(raw_dirs) = std::env::var_os("MOTIONLOOM_FONT_DIRS")
        .or_else(|| std::env::var_os("MOTIONLOOM_FONT_DIR"))
    else {
        return;
    };

    for dir in std::env::split_paths(&raw_dirs) {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
                continue;
            };
            let ext = ext.to_ascii_lowercase();
            if ext == "ttf" || ext == "otf" || ext == "ttc" {
                let _ = font_system.db_mut().load_font_file(&path);
            }
        }
    }
}
