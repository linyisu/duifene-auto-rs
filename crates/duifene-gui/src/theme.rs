use gpui_kit::component::theme::{Theme, ThemeMode, ThemeRegistry};

const PAPER_THEME_JSON: &str = r##"{
  "themes": [
    {
      "name": "Duifene Paper Light",
      "mode": "light",
      "font.size": 15,
      "radius": 10,
      "radius.lg": 14,
      "colors": {
        "background": "#f7f6f2",
        "foreground": "#1c1917",
        "border": "#e7e5de",
        "sidebar.background": "#f1efe9",
        "sidebar.border": "#e7e5de",
        "sidebar.foreground": "#1c1917",
        "title_bar.background": "#f7f6f2",
        "title_bar.border": "#e7e5de",
        "status_bar.background": "#f7f6f2",
        "status_bar.border": "#e7e5de",
        "secondary.background": "#edebe4",
        "secondary.hover.background": "#e5e3da",
        "secondary.active.background": "#ddd9cf",
        "secondary.foreground": "#1c1917",
        "accent.background": "#eae8e0",
        "accent.foreground": "#1c1917",
        "muted.background": "#edebe4",
        "muted.foreground": "#8a857a",
        "primary.background": "#1c1917",
        "primary.hover.background": "#33302b",
        "primary.active.background": "#45413a",
        "primary.foreground": "#faf9f6",
        "input.border": "#dcd9cf",
        "caret": "#1c1917",
        "ring": "#1c191740",
        "selection.background": "#d8d4c8",
        "popover.background": "#ffffff",
        "popover.foreground": "#1c1917",
        "overlay": "#1c19171a",
        "list.even.background": "#faf9f6",
        "list.active.background": "#f0eee7",
        "list.active.border": "#dcd9cf",
        "group_box.background": "#ffffff",
        "group_box.foreground": "#1c1917",
        "success.background": "#16a34a",
        "success.foreground": "#f0fdf4",
        "warning.background": "#d97706",
        "warning.foreground": "#fffbeb",
        "danger.background": "#dc2626",
        "danger.foreground": "#fef2f2",
        "info.background": "#2563eb",
        "info.foreground": "#eff6ff",
        "window.border": "#e7e5de",
        "tab_bar.background": "#f7f6f2",
        "scrollbar.thumb.background": "#dcd9cf",
        "scrollbar.thumb.hover.background": "#c9c5b8",
        "skeleton.background": "#edebe4",
        "switch.background": "#dcd9cf",
        "progress_bar.background": "#1c1917"
      }
    }
  ]
}"##;

pub fn apply_paper_theme(cx: &mut gpui_kit::App) {
    let loaded = ThemeRegistry::global_mut(cx)
        .load_themes_from_str(PAPER_THEME_JSON)
        .is_ok();
    if !loaded {
        eprintln!("theme load failed");
        return;
    }
    if let Some(config) = ThemeRegistry::global(cx)
        .themes()
        .get("Duifene Paper Light")
    {
        Theme::global_mut(cx).light_theme = config.clone();
    }
    Theme::change(ThemeMode::Light, None, cx);
}
