//! The native menu bar. Every command lives here, as Apple's guidelines ask.
//!
//! Keys without a modifier (arrows, space, B) are shown in the title rather than attached as
//! key equivalents, since a menu item would also fire on key repeat and sort a row of photos.
use std::{collections::HashMap, str::FromStr as _};

use eframe::egui::Context;
use muda::{
    AboutMetadata, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem as Native, Submenu,
    accelerator::Accelerator,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    OpenFolder,
    OpenLibrary,
    Reveal,
    Quit,
    Undo,
    SelectAll,
    Keep,
    Discard,
    Skip,
    Restore,
    TrashAll,
    ViewSweep,
    ViewReview,
    ViewKept,
    ToggleZoom,
}

const ITEMS: [(Action, &str, &str, Option<&str>); 15] = [
    (
        Action::OpenFolder,
        "open-folder",
        "Åpne mappe…",
        Some("Cmd+O"),
    ),
    (
        Action::OpenLibrary,
        "open-library",
        "Åpne Bilder-biblioteket",
        Some("Cmd+Shift+O"),
    ),
    (
        Action::Reveal,
        "reveal",
        "Vis i Finder",
        Some("Cmd+Shift+R"),
    ),
    (Action::Quit, "quit", "Avslutt Bildesveip", Some("Cmd+Q")),
    (Action::Undo, "undo", "Angre", Some("Cmd+Z")),
    (Action::SelectAll, "select-all", "Marker alt", Some("Cmd+A")),
    (Action::Keep, "keep", "Behold (→)", None),
    (Action::Discard, "discard", "Kast (←)", None),
    (Action::Skip, "skip", "Hopp over (↓)", None),
    (Action::Restore, "restore", "Legg tilbake (B)", None),
    (
        Action::TrashAll,
        "trash-all",
        "Flytt til papirkurven…",
        Some("Cmd+Shift+Backspace"),
    ),
    (Action::ViewSweep, "view-sweep", "Sveip", Some("Cmd+1")),
    (
        Action::ViewReview,
        "view-review",
        "Til sletting",
        Some("Cmd+2"),
    ),
    (Action::ViewKept, "view-kept", "Beholdt", Some("Cmd+3")),
    (
        Action::ToggleZoom,
        "toggle-zoom",
        "Stor visning (mellomrom)",
        None,
    ),
];

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Enabled {
    pub has_folder: bool,
    pub sweep: bool,
    pub grid_with_selection: bool,
    pub grid: bool,
    pub review_has_photos: bool,
    pub can_undo: bool,
    pub can_reveal: bool,
    pub modal: bool,
    pub library: bool,
}

fn is_enabled(action: Action, state: Enabled) -> bool {
    if state.modal {
        return action == Action::Quit;
    }
    match action {
        Action::OpenFolder | Action::OpenLibrary | Action::Quit => true,
        Action::Reveal => state.can_reveal,
        Action::Undo => state.can_undo,
        Action::SelectAll => state.grid,
        Action::Keep | Action::Discard | Action::Skip => state.sweep,
        Action::Restore => state.grid_with_selection,
        Action::TrashAll => state.review_has_photos,
        Action::ViewSweep | Action::ViewReview | Action::ViewKept | Action::ToggleZoom => {
            state.has_folder
        }
    }
}

pub struct NativeMenu {
    // Kept alive: dropping it would remove the menu.
    _menu: Menu,
    items: HashMap<MenuId, (Action, MenuItem)>,
    last: Option<Enabled>,
}

impl NativeMenu {
    pub fn install(ctx: &Context) -> muda::Result<Self> {
        let mut items = HashMap::new();
        let mut by_action = HashMap::new();
        for (action, id, label, key) in ITEMS {
            let key = key.and_then(|k| Accelerator::from_str(k).ok());
            let item = MenuItem::with_id(id, label, false, key);
            by_action.insert(id, item.clone());
            items.insert(item.id().clone(), (action, item));
        }
        let item = |id: &str| by_action[id].clone();
        let application = Submenu::with_items(
            "Bildesveip",
            true,
            &[
                &Native::about(Some("Om Bildesveip"), Some(AboutMetadata::default())),
                &Native::separator(),
                &Native::services(Some("Tjenester")),
                &Native::separator(),
                &Native::hide(Some("Skjul Bildesveip")),
                &Native::hide_others(Some("Skjul andre")),
                &Native::show_all(Some("Vis alle")),
                &Native::separator(),
                &item("quit"),
            ],
        )?;
        let file = Submenu::with_items(
            "Arkiv",
            true,
            &[&item("open-folder"), &item("open-library"), &item("reveal")],
        )?;
        let edit = Submenu::with_items(
            "Rediger",
            true,
            &[
                &item("undo"),
                &Native::separator(),
                &Native::cut(Some("Klipp ut")),
                &Native::copy(Some("Kopier")),
                &Native::paste(Some("Lim inn")),
                &item("select-all"),
            ],
        )?;
        let image = Submenu::with_items(
            "Bilde",
            true,
            &[
                &item("keep"),
                &item("discard"),
                &item("skip"),
                &item("restore"),
                &item("trash-all"),
            ],
        )?;
        let view = Submenu::with_items(
            "Vis",
            true,
            &[
                &item("view-sweep"),
                &item("view-review"),
                &item("view-kept"),
                &item("toggle-zoom"),
                &Native::separator(),
                &Native::fullscreen(Some("Gå til fullskjerm")),
            ],
        )?;
        let window = Submenu::with_items(
            "Vindu",
            true,
            &[
                &Native::minimize(Some("Minimer")),
                &Native::maximize(Some("Zoom")),
            ],
        )?;
        let menu = Menu::with_items(&[&application, &file, &edit, &image, &view, &window])?;
        menu.init_for_nsapp();
        window.set_as_windows_menu_for_nsapp();

        // A menu click happens while the UI may be idle, so it has to wake it.
        let waker = ctx.clone();
        let forward = move |event: MenuEvent| {
            PENDING.with_lock(|pending| pending.push(event.id));
            waker.request_repaint();
        };
        MenuEvent::set_event_handler(Some(forward));

        Ok(Self {
            _menu: menu,
            items,
            last: None,
        })
    }

    pub fn actions(&self) -> Vec<Action> {
        PENDING
            .with_lock(std::mem::take)
            .into_iter()
            .filter_map(|id| self.items.get(&id).map(|(action, _)| *action))
            .collect()
    }

    pub fn update(&mut self, state: Enabled) {
        if self.last == Some(state) {
            return;
        }
        self.last = Some(state);
        for (action, item) in self.items.values() {
            item.set_enabled(is_enabled(*action, state));
            if *action == Action::TrashAll {
                item.set_text(if state.library {
                    "Slett i Bilder…"
                } else {
                    "Flytt til papirkurven…"
                });
            }
        }
    }
}

struct Pending(std::sync::Mutex<Vec<MenuId>>);

impl Pending {
    fn with_lock<T>(&self, work: impl FnOnce(&mut Vec<MenuId>) -> T) -> T {
        let mut pending = self.0.lock().unwrap_or_else(|e| e.into_inner());
        work(&mut pending)
    }
}

static PENDING: Pending = Pending(std::sync::Mutex::new(Vec::new()));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_modal_dialog_disables_everything_but_quit() {
        let state = Enabled {
            has_folder: true,
            sweep: true,
            modal: true,
            ..Default::default()
        };
        assert!(!is_enabled(Action::Keep, state));
        assert!(!is_enabled(Action::OpenFolder, state));
        assert!(is_enabled(Action::Quit, state));
    }

    #[test]
    fn moving_to_the_trash_needs_marked_photos() {
        assert!(!is_enabled(Action::TrashAll, Enabled::default()));
        let state = Enabled {
            review_has_photos: true,
            ..Default::default()
        };
        assert!(is_enabled(Action::TrashAll, state));
    }
}
