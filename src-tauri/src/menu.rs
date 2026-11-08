use serde::Deserialize;
use std::collections::HashMap;
use tauri::{
    AppHandle, Manager,
    menu::{Menu, MenuItem, PredefinedMenuItem as Native, Submenu},
};
pub struct MenuState(HashMap<&'static str, MenuItem<tauri::Wry>>);
#[derive(Clone, Copy, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum MenuContext {
    Empty,
    Sweep,
    Review,
    Kept,
}
fn enabled(
    id: &str,
    context: MenuContext,
    can_undo: bool,
    selection: bool,
    count: usize,
    can_reveal: bool,
) -> bool {
    match id {
        "open-folder" | "open-library" => true,
        "undo" => can_undo,
        "reveal" => can_reveal,
        "keep" | "discard" | "skip" => context == MenuContext::Sweep,
        "restore" => matches!(context, MenuContext::Review | MenuContext::Kept) && selection,
        "select-all" => matches!(context, MenuContext::Review | MenuContext::Kept),
        "trash-all" => context == MenuContext::Review && count > 0,
        _ => context != MenuContext::Empty,
    }
}
#[tauri::command]
pub fn set_menu_context(
    app: AppHandle,
    context: MenuContext,
    can_undo: bool,
    has_selection: bool,
    discard_count: usize,
    can_reveal: bool,
) -> Result<(), String> {
    let state = app.state::<MenuState>();
    for (id, item) in &state.0 {
        item.set_enabled(enabled(
            id,
            context,
            can_undo,
            has_selection,
            discard_count,
            can_reveal,
        ))
        .map_err(|e| format!("Kan ikke oppdatere menyen: {e}"))?;
    }
    Ok(())
}
pub fn build(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let mut items = HashMap::new();
    for (id, label, key) in [
        ("open-folder", "Åpne mappe…", "Cmd+O"),
        ("open-library", "Åpne Bilder-biblioteket", "Cmd+Shift+O"),
        ("reveal", "Vis i Finder", "Cmd+Shift+R"),
        ("undo", "Angre", "Cmd+Z"),
        ("select-all", "Marker alt", "Cmd+A"),
        ("keep", "Behold", "ArrowRight"),
        ("discard", "Kast", "ArrowLeft"),
        ("skip", "Hopp over", "ArrowDown"),
        ("restore", "Legg tilbake", "B"),
        ("trash-all", "Flytt til papirkurven…", "Cmd+Shift+Backspace"),
        ("view-sweep", "Sveip", "Cmd+1"),
        ("view-review", "Til sletting", "Cmd+2"),
        ("view-kept", "Beholdt", "Cmd+3"),
        ("toggle-zoom", "Stor visning", "Space"),
    ] {
        items.insert(
            id,
            MenuItem::with_id(
                app,
                id,
                label,
                matches!(id, "open-folder" | "open-library"),
                Some(key),
            )?,
        );
    }
    let application = Submenu::with_items(
        app,
        "Bildesveip",
        true,
        &[
            &Native::about(app, Some("Om Bildesveip"), None)?,
            &Native::separator(app)?,
            &Native::services(app, Some("Tjenester"))?,
            &Native::separator(app)?,
            &Native::hide(app, Some("Skjul Bildesveip"))?,
            &Native::hide_others(app, Some("Skjul andre"))?,
            &Native::show_all(app, Some("Vis alle"))?,
            &Native::separator(app)?,
            &Native::quit(app, Some("Avslutt Bildesveip"))?,
        ],
    )?;
    let file = Submenu::with_items(
        app,
        "Arkiv",
        true,
        &[
            &items["open-folder"],
            &items["open-library"],
            &items["reveal"],
        ],
    )?;
    let edit = Submenu::with_items(
        app,
        "Rediger",
        true,
        &[
            &items["undo"],
            &Native::separator(app)?,
            &Native::cut(app, Some("Klipp ut"))?,
            &Native::copy(app, Some("Kopier"))?,
            &Native::paste(app, Some("Lim inn"))?,
            &items["select-all"],
        ],
    )?;
    let image = Submenu::with_items(
        app,
        "Bilde",
        true,
        &[
            &items["keep"],
            &items["discard"],
            &items["skip"],
            &items["restore"],
            &items["trash-all"],
        ],
    )?;
    let view = Submenu::with_items(
        app,
        "Vis",
        true,
        &[
            &items["view-sweep"],
            &items["view-review"],
            &items["view-kept"],
            &items["toggle-zoom"],
            &Native::separator(app)?,
            &Native::fullscreen(app, Some("Gå til fullskjerm"))?,
        ],
    )?;
    let window = Submenu::with_items(
        app,
        "Vindu",
        true,
        &[
            &Native::minimize(app, Some("Minimer"))?,
            &Native::maximize(app, Some("Zoom"))?,
        ],
    )?;
    let menu = Menu::with_items(app, &[&application, &file, &edit, &image, &view, &window])?;
    app.manage(MenuState(items));
    Ok(menu)
}
pub fn is_action(app: &AppHandle, id: &str) -> bool {
    app.state::<MenuState>().0.contains_key(id)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn contexts() {
        let on = |id, context, selection, count, reveal| {
            enabled(id, context, false, selection, count, reveal)
        };
        assert!(on("open-folder", MenuContext::Empty, false, 0, false));
        assert!(on("open-library", MenuContext::Empty, false, 0, false));
        assert!(!on("trash-all", MenuContext::Sweep, true, 10, false));
        assert!(on("trash-all", MenuContext::Review, false, 1, false));
        assert!(!on("restore", MenuContext::Kept, false, 1, false));
        assert!(on("restore", MenuContext::Kept, true, 1, false));
        assert!(on("reveal", MenuContext::Sweep, false, 0, true));
        assert!(!on("reveal", MenuContext::Sweep, false, 0, false));
    }
}
