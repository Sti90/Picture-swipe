//! Window layout, commands and everything shared by the views.
use std::{path::PathBuf, time::Instant};

use eframe::egui::{
    self, Align2, Color32, Context, CornerRadius, Event, Key, Modifiers, Rect, Sense, Stroke, Ui,
    ViewportCommand, pos2, vec2,
};

use crate::{
    backend::{self, Backend, Paths, Reply},
    decisions::Decision,
    format::{bytes, count, images, photo_title, when},
    grid::Grid,
    images::Images,
    library::{self, Status},
    menu::{Action, Enabled, NativeMenu},
    store::{Sheet, Store, View},
    sweep::{Sweep, draw_photo},
    trash::TrashResult,
    widgets::{self, Icon, Palette, RowContent, Style, push_button, sidebar_row, tool_button},
};

const SIDEBAR: f32 = 220.0;
const TOOLBAR: f32 = 52.0;
const STATUS_BAR: f32 = 34.0;
const PRELOAD_AHEAD: usize = 3;
const DEDUPE_SECONDS: f64 = 0.1;

#[derive(Clone, Copy)]
pub struct SystemSettings {
    pub accent: Color32,
    pub reduce_motion: bool,
}

pub struct App {
    pub store: Store,
    pub images: Images,
    pub sweep: Sweep,
    pub grid: Grid,
    pub settings: SystemSettings,
    backend: Option<Backend>,
    menu: Option<NativeMenu>,
    cache: PathBuf,
    sheet_focus: Option<usize>,
    last_action: Option<(Action, f64)>,
    title: String,
    wants_folder_dialog: bool,
    quit_when_idle: bool,
    started: Instant,
}

impl App {
    pub fn new(
        ctx: &Context,
        paths: Paths,
        menu: NativeMenu,
        settings: SystemSettings,
    ) -> Result<Self, String> {
        let cache = paths.cache.clone();
        let backend = Backend::start(ctx.clone(), paths);
        let library = backend.store.library.clone();
        let images = Images::start(ctx, library, cache.clone())?;
        let mut store = Store::default();
        if let Some(path) = backend.store.last_folder() {
            store.open(path);
        }
        Ok(Self {
            store,
            images,
            sweep: Sweep::default(),
            grid: Grid::default(),
            settings,
            backend: Some(backend),
            menu: Some(menu),
            cache,
            sheet_focus: None,
            last_action: None,
            title: String::new(),
            wants_folder_dialog: false,
            quit_when_idle: false,
            started: Instant::now(),
        })
    }

    /// Finishes every queued write. Called when the app quits, so a window close has
    /// already waited for anything that needs a system dialog on the main thread.
    pub fn finish(&mut self) {
        self.flush_outbox();
        discard_images(std::mem::replace(&mut self.images, Images::idle()));
        if let Some(backend) = &mut self.backend {
            backend.shutdown();
        }
    }

    fn flush_outbox(&mut self) {
        let commands = std::mem::take(&mut self.store.outbox);
        if let Some(backend) = &self.backend {
            for command in commands {
                backend.send(command);
            }
        }
    }

    fn receive(&mut self, ctx: &Context) {
        let Some(backend) = &self.backend else { return };
        let replies: Vec<Reply> = backend.replies.try_iter().collect();
        for reply in replies {
            match reply {
                Reply::Opened {
                    request,
                    result,
                    access,
                } => {
                    if self.store.on_opened(request, result, access) {
                        self.install_new_folder(ctx);
                    }
                }
                Reply::Written { token, result } => self.store.on_written(token, result),
                Reply::Trashed { result } => self.on_trashed(result),
            }
        }
    }

    fn on_trashed(&mut self, result: Result<TrashResult, String>) {
        self.store.on_trashed(result);
        self.sheet_focus = None;
    }

    /// PhotoKit shows its dialogs on the main thread, so quitting must not wait for them there.
    fn waiting_on_system(&self) -> bool {
        self.store.loading || matches!(self.store.sheet, Some(Sheet::Confirm { busy: true, .. }))
    }

    fn handle_quit(&mut self, ctx: &Context) {
        let busy = self.waiting_on_system();
        if busy && ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.quit_when_idle = true;
        }
        if self.quit_when_idle && !busy {
            self.quit_when_idle = false;
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
    }

    /// Photos are numbered per folder, so nothing from the last one may be reused. The old
    /// images always go, even if the new loader can't start: then nothing can be judged.
    fn install_new_folder(&mut self, ctx: &Context) {
        self.sweep = Sweep::default();
        self.grid = Grid::default();
        discard_images(std::mem::replace(&mut self.images, Images::idle()));
        let Some(backend) = &self.backend else { return };
        let library = backend.store.library.clone();
        match Images::start(ctx, library, self.cache.clone()) {
            Ok(images) => self.images = images,
            Err(error) => self.store.error = Some(error),
        }
    }

    pub fn pick_folder(&mut self) {
        self.wants_folder_dialog = true;
    }

    fn run_folder_dialog(&mut self) {
        if !std::mem::take(&mut self.wants_folder_dialog) || self.store.locked() {
            return;
        }
        if let Some(folder) = rfd::FileDialog::new().set_title("Velg mappe").pick_folder() {
            self.store.open(folder.to_string_lossy().into_owned());
        }
    }

    fn zoom_target(&self) -> Option<usize> {
        if self.store.view == View::Sweep {
            self.store.current()
        } else {
            self.store
                .cursor
                .or_else(|| self.store.selection.iter().next().copied())
        }
    }

    fn can_reveal(&self) -> bool {
        if self.store.folder.is_none() || self.store.is_library() {
            return false;
        }
        if self.store.view == View::Sweep {
            self.store.current().is_some()
        } else {
            !self.store.selection.is_empty()
        }
    }

    fn run(&mut self, ctx: &Context, action: Action, now: f64) {
        // The frame clock stands still while the window is hidden, so this needs a real one.
        let clock = self.started.elapsed().as_secs_f64();
        self.run_at(ctx, action, now, clock);
    }

    fn run_at(&mut self, ctx: &Context, action: Action, now: f64, clock: f64) {
        // The same keypress can reach us from the keyboard and from the native menu.
        if self
            .last_action
            .is_some_and(|(last, at)| last == action && clock - at < DEDUPE_SECONDS)
        {
            return;
        }
        self.last_action = Some((action, clock));
        if action == Action::Quit {
            ctx.send_viewport_cmd(ViewportCommand::Close);
            return;
        }
        if self.store.sheet.is_some() {
            return;
        }
        let width = ctx.content_rect().width();
        match action {
            Action::OpenFolder => self.pick_folder(),
            Action::OpenLibrary => self.store.open(library::LIBRARY_PATH.to_owned()),
            Action::Reveal if self.can_reveal() => {
                if let Some(photo) = self.zoom_target() {
                    backend::reveal_in_finder(&self.store.photos()[photo].path);
                }
            }
            Action::Undo => {
                let restored = self.store.undo();
                self.undo_sweep_animation(&restored, width, now);
            }
            Action::SelectAll if self.store.view != View::Sweep => self.select_all(),
            Action::Keep if self.store.view == View::Sweep => self.choose(true, now),
            Action::Discard if self.store.view == View::Sweep => self.choose(false, now),
            Action::Skip if self.store.view == View::Sweep => self.skip(now),
            Action::Restore => self.store.restore_selected(),
            Action::TrashAll => self.store.ask_trash(),
            Action::ViewSweep => self.store.set_view(View::Sweep),
            Action::ViewReview => self.store.set_view(View::Review),
            Action::ViewKept => self.store.set_view(View::Kept),
            Action::ToggleZoom if self.store.folder.is_some() => {
                self.store.zoom = !self.store.zoom && self.zoom_target().is_some();
            }
            _ => {}
        }
    }

    fn handle_keys(&mut self, ctx: &Context, now: f64) {
        let presses: Vec<(Key, Modifiers, bool)> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|event| match event {
                    Event::Key {
                        key,
                        pressed: true,
                        repeat,
                        modifiers,
                        ..
                    } => Some((*key, *modifiers, *repeat)),
                    _ => None,
                })
                .collect()
        });
        for (key, modifiers, repeat) in presses {
            if self.store.sheet.is_some() {
                self.sheet_key(key, modifiers, repeat);
                continue;
            }
            if self.store.loading {
                continue;
            }
            if modifiers.command {
                if !repeat {
                    self.command_key(ctx, key, modifiers, now);
                }
                continue;
            }
            if self.store.folder.is_none() {
                continue;
            }
            if key == Key::Escape && !repeat {
                if self.store.zoom {
                    self.store.zoom = false;
                } else if self.store.view != View::Sweep {
                    self.store.clear_selection();
                }
                continue;
            }
            if key == Key::Space {
                if !repeat {
                    self.run(ctx, Action::ToggleZoom, now);
                }
                continue;
            }
            if self.store.view == View::Sweep {
                // Holding an arrow must not sort a row of photos.
                if !repeat {
                    match key {
                        Key::ArrowRight => self.run(ctx, Action::Keep, now),
                        Key::ArrowLeft => self.run(ctx, Action::Discard, now),
                        Key::ArrowDown => self.run(ctx, Action::Skip, now),
                        _ => {}
                    }
                }
            } else {
                match key {
                    Key::ArrowLeft => self.move_cursor(-1, 0, modifiers.shift),
                    Key::ArrowRight => self.move_cursor(1, 0, modifiers.shift),
                    Key::ArrowUp => self.move_cursor(0, -1, modifiers.shift),
                    Key::ArrowDown => self.move_cursor(0, 1, modifiers.shift),
                    Key::B if !repeat => self.run(ctx, Action::Restore, now),
                    _ => {}
                }
            }
        }
    }

    fn command_key(&mut self, ctx: &Context, key: Key, modifiers: Modifiers, now: f64) {
        let action = match (key, modifiers.shift) {
            (Key::O, false) => Action::OpenFolder,
            (Key::O, true) => Action::OpenLibrary,
            (Key::Z, false) => Action::Undo,
            (Key::Num1, _) => Action::ViewSweep,
            (Key::Num2, _) => Action::ViewReview,
            (Key::Num3, _) => Action::ViewKept,
            (Key::R, true) => Action::Reveal,
            (Key::A, false) => Action::SelectAll,
            (Key::Backspace, true) => Action::TrashAll,
            _ => return,
        };
        self.run(ctx, action, now);
    }

    fn sheet_key(&mut self, key: Key, modifiers: Modifiers, repeat: bool) {
        if repeat {
            return;
        }
        let buttons = match self.store.sheet {
            Some(Sheet::Confirm { .. }) => 2,
            _ => 1,
        };
        match key {
            Key::Escape => self.close_sheet(),
            Key::Period if modifiers.command => self.close_sheet(),
            Key::Tab => {
                let step = if modifiers.shift { buttons - 1 } else { 1 };
                self.sheet_focus =
                    Some(self.sheet_focus.map_or(0, |focus| (focus + step) % buttons));
            }
            // No default button: Return only acts on what the keyboard has focused.
            Key::Enter | Key::Space => match (self.sheet_focus, buttons) {
                (Some(0), 2) => self.close_sheet(),
                (Some(1), 2) => self.store.confirm_trash(),
                (Some(_), _) => self.close_sheet(),
                _ => {}
            },
            _ => {}
        }
    }

    fn close_sheet(&mut self) {
        self.store.close_sheet();
        self.sheet_focus = None;
    }

    /// Runs before every frame, and also while the window is hidden or minimized, when
    /// nothing is drawn. So quitting and finished background work don't depend on drawing.
    pub fn tick(&mut self, ctx: &Context) {
        let now = ctx.input(|i| i.time);
        self.sweep.acted = false;
        self.receive(ctx);
        if self.store.sheet.is_none() {
            self.sheet_focus = None;
        }
        self.handle_quit(ctx);
        if let Some(menu) = &self.menu {
            for action in menu.actions() {
                self.run(ctx, action, now);
            }
        }
        self.run_folder_dialog();
        self.flush_outbox();
        self.sync_menu();
        self.sync_title(ctx);
    }

    pub fn show(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        let full = ctx.content_rect();
        let palette = Palette::new(ctx.theme(), self.settings.accent);
        ui.painter()
            .rect_filled(full, CornerRadius::ZERO, palette.background);

        self.handle_keys(&ctx, now);

        let dropped = ctx.input(|i| i.raw.dropped_files.first().map(|f| f.path().to_path_buf()));
        if let Some(path) = dropped.filter(|p| p.is_dir()) {
            self.store.open(path.to_string_lossy().into_owned());
        }

        self.sync_images(&ctx);

        if self.store.folder.is_some() {
            self.sidebar(ui, full, palette);
            self.content(ui, full, palette);
        } else {
            self.empty_state(ui, full, palette);
        }
        self.lightbox(ui, full);
        self.sheet(ui, full, palette);
        if ctx.input(|i| !i.raw.hovered_files.is_empty()) {
            ui.painter().rect_stroke(
                full.shrink(3.0),
                CornerRadius::same(10),
                Stroke::new(3.0, palette.accent),
                egui::StrokeKind::Inside,
            );
        }

        self.run_folder_dialog();
        self.flush_outbox();
        self.sync_menu();
        self.sync_title(&ctx);
    }

    fn sync_images(&mut self, ctx: &Context) {
        let zoomed = if self.store.zoom {
            self.zoom_target()
        } else {
            None
        };
        let mut previews: Vec<usize> = Vec::new();
        previews.extend(zoomed);
        if self.store.view == View::Sweep {
            previews.extend(self.store.queue.iter().take(PRELOAD_AHEAD + 1).copied());
        }
        previews.dedup();
        let store = &self.store;
        let thumbnails = if store.view == View::Sweep {
            &[][..]
        } else {
            self.grid.visible()
        };
        self.images.sync(ctx, &previews, thumbnails, |photo| {
            store.photos()[photo].path.clone()
        });
    }

    fn sync_menu(&mut self) {
        let view = self.store.view;
        let has_folder = self.store.folder.is_some();
        let discard = self.store.counts().2;
        let state = Enabled {
            has_folder,
            sweep: has_folder && view == View::Sweep,
            grid_with_selection: has_folder
                && view != View::Sweep
                && !self.store.selection.is_empty(),
            grid: has_folder && view != View::Sweep,
            review_has_photos: has_folder && view == View::Review && discard > 0,
            can_undo: self.store.can_undo(),
            can_reveal: self.can_reveal(),
            modal: self.store.sheet.is_some(),
            library: self.store.is_library(),
        };
        if let Some(menu) = &mut self.menu {
            menu.update(state);
        }
    }

    fn sync_title(&mut self, ctx: &Context) {
        let title = self
            .store
            .folder
            .as_ref()
            .map_or("Bildesveip", |f| f.folder_name.as_str());
        if title != self.title {
            self.title = title.to_owned();
            ctx.send_viewport_cmd(ViewportCommand::Title(self.title.clone()));
        }
    }

    fn window_drag(&self, ui: &Ui, rect: Rect, id: &str) {
        if ui
            .interact(rect, ui.scope_id().with(id), Sense::drag())
            .drag_started()
        {
            ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
        }
    }

    fn sidebar(&mut self, ui: &mut Ui, full: Rect, palette: Palette) {
        let rect = Rect::from_min_size(full.min, vec2(SIDEBAR, full.height()));
        let painter = ui.painter().clone();
        painter.rect_filled(rect, CornerRadius::ZERO, palette.sidebar);
        painter.vline(
            rect.right(),
            rect.y_range(),
            Stroke::new(1.0, palette.separator),
        );
        self.window_drag(
            ui,
            Rect::from_min_size(rect.min, vec2(SIDEBAR, TOOLBAR)),
            "sidebar-drag",
        );

        let name = self
            .store
            .folder
            .as_ref()
            .map(|f| f.folder_name.clone())
            .unwrap_or_default();
        let name = widgets::fit_text(ui, &name, 11.0, SIDEBAR - 32.0);
        widgets::text(
            &painter,
            pos2(rect.left() + 16.0, rect.top() + TOOLBAR + 8.0),
            Align2::LEFT_CENTER,
            name,
            11.0,
            palette.secondary,
        );

        let (undecided, keep, discard) = self.store.counts();
        let rows = [
            (View::Sweep, "Sveip", Icon::Sweep, undecided),
            (View::Review, "Til sletting", Icon::Trash, discard),
            (View::Kept, "Beholdt", Icon::Check, keep),
        ];
        let enabled = !self.store.locked();
        let mut y = rect.top() + TOOLBAR + 22.0;
        for (view, label, kind, number) in rows {
            let row = Rect::from_min_size(pos2(rect.left() + 8.0, y), vec2(SIDEBAR - 16.0, 28.0));
            let content = RowContent {
                kind,
                label,
                count: Some(count(number)),
                selected: self.store.view == view,
            };
            if sidebar_row(ui, row, label, content, enabled, palette) {
                self.store.set_view(view);
            }
            y += 30.0;
        }
        let mut y = rect.bottom() - 74.0;
        for (id, label, kind) in [
            ("library", "Åpne Bilder-biblioteket", Icon::Library),
            ("folder", "Åpne mappe…", Icon::Folder),
        ] {
            let row = Rect::from_min_size(pos2(rect.left() + 8.0, y), vec2(SIDEBAR - 16.0, 28.0));
            let content = RowContent {
                kind,
                label,
                count: None,
                selected: false,
            };
            if sidebar_row(ui, row, id, content, enabled, palette) {
                if id == "library" {
                    self.store.open(library::LIBRARY_PATH.to_owned());
                } else {
                    self.pick_folder();
                }
            }
            y += 30.0;
        }
    }

    fn content(&mut self, ui: &mut Ui, full: Rect, palette: Palette) {
        let rect = Rect::from_min_max(pos2(full.left() + SIDEBAR, full.top()), full.max);
        self.toolbar(ui, rect, palette);
        let body = Rect::from_min_max(pos2(rect.left(), rect.top() + TOOLBAR), rect.max);
        if self.store.view == View::Sweep {
            self.show_sweep(ui, body, palette);
        } else {
            let grid_area =
                Rect::from_min_max(body.min, pos2(body.right(), body.bottom() - STATUS_BAR));
            self.show_grid(ui, grid_area, palette);
            let bar = Rect::from_min_max(pos2(body.left(), grid_area.bottom()), body.max);
            ui.painter().hline(
                bar.x_range(),
                bar.top(),
                Stroke::new(1.0, palette.separator),
            );
            self.status_line(ui, bar.shrink2(vec2(16.0, 3.0)), palette);
        }
    }

    fn toolbar(&mut self, ui: &mut Ui, rect: Rect, palette: Palette) {
        let bar = Rect::from_min_size(rect.min, vec2(rect.width(), TOOLBAR));
        self.window_drag(ui, bar, "toolbar-drag");
        let (title, subtitle) = self.toolbar_titles();
        let painter = ui.painter().clone();
        let left = bar.left() + 20.0;
        if subtitle.is_empty() {
            widgets::text(
                &painter,
                pos2(left, bar.center().y),
                Align2::LEFT_CENTER,
                title,
                15.0,
                palette.label,
            );
        } else {
            widgets::text(
                &painter,
                pos2(left, bar.center().y - 8.0),
                Align2::LEFT_CENTER,
                title,
                15.0,
                palette.label,
            );
            widgets::text(
                &painter,
                pos2(left, bar.center().y + 10.0),
                Align2::LEFT_CENTER,
                subtitle,
                11.0,
                palette.secondary,
            );
        }

        let enabled = !self.store.locked();
        let mut x = bar.right() - 16.0;
        let (_, _, discard) = self.store.counts();
        if self.store.view == View::Review {
            let label = if self.store.is_library() {
                "Slett i Bilder…"
            } else {
                "Flytt til papirkurven…"
            };
            let width = widgets::button_width(ui, label);
            let rect =
                Rect::from_min_size(pos2(x - width, bar.center().y - 14.0), vec2(width, 28.0));
            if push_button(
                ui,
                rect,
                "trash-all",
                label,
                Style::Prominent,
                enabled && discard > 0,
                false,
                palette,
            ) {
                self.store.ask_trash();
            }
            x -= width + 12.0;
        }
        let tool =
            |x: f32| Rect::from_center_size(pos2(x - 14.0, bar.center().y), vec2(28.0, 28.0));
        let now = ui.input(|i| i.time);
        let can_undo = enabled && self.store.can_undo();
        if tool_button(
            ui,
            tool(x),
            "undo",
            Icon::Undo,
            "Angre (⌘Z)",
            can_undo,
            palette,
        ) {
            let ctx = ui.ctx().clone();
            self.run(&ctx, Action::Undo, now);
        }
        x -= 32.0;
        if !self.store.is_library() {
            let can_reveal = enabled && self.can_reveal();
            if tool_button(
                ui,
                tool(x),
                "reveal",
                Icon::Reveal,
                "Vis i Finder",
                can_reveal,
                palette,
            ) {
                let ctx = ui.ctx().clone();
                self.run(&ctx, Action::Reveal, now);
            }
        }
    }

    fn toolbar_titles(&mut self) -> (&'static str, String) {
        let library = self.store.is_library();
        let sized = |text: String, size: u64| {
            if library {
                text
            } else {
                format!("{text}, {}", bytes(size))
            }
        };
        match self.store.view {
            View::Sweep => {
                let total = self.store.total_photos();
                let left = self.store.queue.len();
                let subtitle = if left > 0 {
                    format!("{} av {}", count(total - left + 1), count(total))
                } else {
                    String::new()
                };
                ("Sveip", subtitle)
            }
            view => {
                let decision = view.decision();
                let number = self.store.list(view).len();
                let size = self.store.total_bytes(decision);
                let subtitle = if number > 0 {
                    sized(images(number), size)
                } else {
                    String::new()
                };
                (
                    if decision == Decision::Keep {
                        "Beholdt"
                    } else {
                        "Til sletting"
                    },
                    subtitle,
                )
            }
        }
    }

    /// Errors, confirmations, or quiet details about what's selected.
    pub fn status_line(&mut self, ui: &mut Ui, rect: Rect, palette: Palette) {
        let painter = ui.painter().clone();
        let y = rect.center().y;
        let close = Rect::from_center_size(pos2(rect.right() - 11.0, y), vec2(22.0, 22.0));
        let dismissable = self.store.library_access.is_some()
            || self.store.error.is_some()
            || self.store.status.is_some();
        let mut right = rect.right();
        if dismissable {
            right -= 26.0;
            if tool_button(ui, close, "dismiss", Icon::Close, "Lukk", true, palette) {
                self.store.error = None;
                self.store.status = None;
                self.store.library_access = None;
                return;
            }
        }
        let flagged = |ui: &Ui, right: f32, kind: Icon, message: &str, color: Color32| {
            widgets::icon(&painter, pos2(rect.left() + 8.0, y), kind, color);
            let fitted = widgets::fit_text(ui, message, 12.0, right - rect.left() - 24.0);
            widgets::text(
                &painter,
                pos2(rect.left() + 22.0, y),
                Align2::LEFT_CENTER,
                fitted,
                12.0,
                color,
            );
        };
        if let Some(access) = self.store.library_access {
            let label = "Åpne Systeminnstillinger";
            let width = widgets::button_width(ui, label);
            let button = Rect::from_min_size(pos2(right - width, y - 12.0), vec2(width, 24.0));
            if push_button(
                ui,
                button,
                "privacy",
                label,
                Style::Plain,
                true,
                false,
                palette,
            ) {
                backend::open_photos_privacy_settings();
            }
            flagged(
                ui,
                right - width - 8.0,
                Icon::Warning,
                access_help(access),
                palette.error,
            );
        } else if let Some(error) = self.store.error.clone() {
            flagged(ui, right, Icon::Warning, &error, palette.error);
        } else if let Some(status) = self.store.status.clone() {
            flagged(ui, right, Icon::Check, &status, palette.secondary);
        } else if !self.store.selection.is_empty() {
            let selection = self.selection_details();
            let details = widgets::fit_text(ui, &selection, 12.0, rect.width());
            widgets::text(
                &painter,
                pos2(rect.left(), y),
                Align2::LEFT_CENTER,
                details,
                12.0,
                palette.secondary,
            );
        }
    }

    fn selection_details(&self) -> String {
        let photos = self.store.photos();
        let selected = &self.store.selection;
        let mut details = format!("{} valgt", count(selected.len()));
        if !self.store.is_library() {
            let size = selected.iter().map(|p| photos[*p].size).sum();
            details = format!("{details}, {}", bytes(size));
        }
        if selected.len() == 1
            && let Some(photo) = selected.iter().next()
        {
            let photo = &photos[*photo];
            details = format!("{details}    {}", photo_title(photo));
            if !self.store.is_library() {
                details = format!("{details}    {}", when(photo.modified));
            }
        }
        details
    }

    fn empty_state(&mut self, ui: &mut Ui, full: Rect, palette: Palette) {
        self.window_drag(
            ui,
            Rect::from_min_size(full.min, vec2(full.width(), TOOLBAR)),
            "empty-drag",
        );
        let painter = ui.painter().clone();
        let center = full.center();
        widgets::text(
            &painter,
            center - vec2(0.0, 52.0),
            Align2::CENTER_CENTER,
            "Sorter bildene dine",
            17.0,
            palette.label,
        );
        widgets::text(
            &painter,
            center - vec2(0.0, 24.0),
            Align2::CENTER_CENTER,
            "Velg en mappe, eller slipp den her. Ingenting slettes før du har gått gjennom bildene.",
            13.0,
            palette.secondary,
        );
        let loading = self.store.loading;
        let library_label = "Åpne Bilder-biblioteket";
        let library_width = widgets::button_width(ui, library_label);
        let folder_width = 118.0;
        let total = library_width + 12.0 + folder_width;
        let y = center.y + 16.0;
        let first = Rect::from_min_size(
            pos2(center.x - total / 2.0, y - 14.0),
            vec2(library_width, 28.0),
        );
        let second = Rect::from_min_size(
            pos2(first.right() + 12.0, y - 14.0),
            vec2(folder_width, 28.0),
        );
        if push_button(
            ui,
            first,
            "open-library",
            library_label,
            Style::Plain,
            !loading,
            false,
            palette,
        ) {
            self.store.open(library::LIBRARY_PATH.to_owned());
        }
        if push_button(
            ui,
            second,
            "open-folder",
            "Velg mappe…",
            Style::Prominent,
            !loading,
            false,
            palette,
        ) {
            self.pick_folder();
        }
        if loading {
            egui::Spinner::new().paint_at(
                ui,
                Rect::from_center_size(center + vec2(0.0, 66.0), egui::Vec2::splat(20.0)),
            );
        }
        let line = center.y + 60.0;
        if let Some(error) = self.store.error.clone() {
            widgets::text(
                &painter,
                pos2(center.x, line),
                Align2::CENTER_CENTER,
                error,
                13.0,
                palette.error,
            );
        }
        if let Some(access) = self.store.library_access {
            widgets::text(
                &painter,
                pos2(center.x, line),
                Align2::CENTER_CENTER,
                access_help(access),
                13.0,
                palette.error,
            );
            let label = "Åpne Systeminnstillinger";
            let rect = Rect::from_center_size(
                pos2(center.x, line + 34.0),
                vec2(widgets::button_width(ui, label), 28.0),
            );
            if push_button(
                ui,
                rect,
                "privacy",
                label,
                Style::Plain,
                true,
                false,
                palette,
            ) {
                backend::open_photos_privacy_settings();
            }
        }
    }

    fn lightbox(&mut self, ui: &mut Ui, full: Rect) {
        if !self.store.zoom {
            return;
        }
        let Some(photo) = self.zoom_target() else {
            self.store.zoom = false;
            return;
        };
        let response = ui.interact(full, ui.scope_id().with("lightbox"), Sense::click());
        let painter = ui.painter().clone();
        painter.rect_filled(full, CornerRadius::ZERO, Color32::from_black_alpha(238));
        let area = full.shrink2(vec2(24.0, 40.0));
        if let Some(texture) = self.images.preview(photo) {
            draw_photo(&painter, texture, area, 0.0, 0.0, 1.0, 1.0);
        } else if self.images.preview_failed(photo) {
            let title = photo_title(&self.store.photos()[photo]);
            widgets::text(
                &painter,
                area.center(),
                Align2::CENTER_CENTER,
                title,
                13.0,
                Color32::from_white_alpha(150),
            );
        } else {
            egui::Spinner::new().color(Color32::WHITE).paint_at(
                ui,
                Rect::from_center_size(area.center(), egui::Vec2::splat(24.0)),
            );
        }
        if response.clicked() {
            self.store.zoom = false;
        }
    }

    fn sheet(&mut self, ui: &mut Ui, full: Rect, palette: Palette) {
        let Some(sheet) = self.store.sheet.clone() else {
            return;
        };
        // Takes every click, so nothing behind the dialog can be reached.
        let _ = ui.interact(
            full,
            ui.scope_id().with("sheet-backdrop"),
            Sense::click_and_drag(),
        );
        let painter = ui.painter().clone();
        painter.rect_filled(full, CornerRadius::ZERO, Color32::from_black_alpha(90));
        let width = 440.0;
        let wrap = width - 48.0;
        let (title, body, confirm, failures) = self.sheet_copy(&sheet);
        let title_galley = painter.layout(title, widgets::font(15.0), palette.label, wrap);
        let body_galley = painter.layout(body, widgets::font(13.0), palette.secondary, wrap);
        let list_height = (failures.len().min(6) as f32) * 38.0;
        let height = 24.0
            + title_galley.size().y
            + 8.0
            + body_galley.size().y
            + 16.0
            + list_height
            + 28.0
            + 20.0;
        let rect = Rect::from_center_size(full.center(), vec2(width, height));
        let sheet_fill = if ui.visuals().dark_mode {
            Color32::from_rgb(0x2c, 0x2c, 0x2e)
        } else {
            Color32::from_rgb(0xfb, 0xfb, 0xfd)
        };
        painter.add(
            egui::epaint::Shadow {
                offset: [0, 12],
                blur: 40,
                spread: 0,
                color: Color32::from_black_alpha(90),
            }
            .as_shape(rect, CornerRadius::same(12)),
        );
        painter.rect_filled(rect, CornerRadius::same(12), sheet_fill);
        let x = rect.left() + 24.0;
        let mut y = rect.top() + 24.0 + title_galley.size().y + 8.0;
        painter.galley(pos2(x, rect.top() + 24.0), title_galley, palette.label);
        let body_size = body_galley.size().y;
        painter.galley(pos2(x, y), body_galley, palette.secondary);
        y += body_size + 16.0;
        for (name, why) in failures.iter().take(6) {
            let fitted = widgets::fit_text(ui, name, 12.0, wrap);
            widgets::text(
                &painter,
                pos2(x, y),
                Align2::LEFT_TOP,
                fitted,
                12.0,
                palette.label,
            );
            let reason = widgets::fit_text(ui, why, 11.0, wrap);
            widgets::text(
                &painter,
                pos2(x, y + 16.0),
                Align2::LEFT_TOP,
                reason,
                11.0,
                palette.secondary,
            );
            y += 38.0;
        }
        let buttons_y = rect.bottom() - 20.0 - 28.0;
        let focus = self.sheet_focus;
        match (&sheet, confirm) {
            (Sheet::Confirm { busy, .. }, Some(action)) => {
                let action = if *busy {
                    "Flytter…".to_owned()
                } else {
                    action
                };
                let action_width = widgets::button_width(ui, &action).max(112.0);
                let action_rect = Rect::from_min_size(
                    pos2(rect.right() - 24.0 - action_width, buttons_y),
                    vec2(action_width, 28.0),
                );
                let cancel_rect = Rect::from_min_size(
                    pos2(action_rect.left() - 8.0 - 80.0, buttons_y),
                    vec2(80.0, 28.0),
                );
                if push_button(
                    ui,
                    cancel_rect,
                    "sheet-cancel",
                    "Avbryt",
                    Style::Plain,
                    !*busy,
                    focus == Some(0),
                    palette,
                ) {
                    self.close_sheet();
                }
                // Not the prominent style: nothing here should look like the default choice.
                if push_button(
                    ui,
                    action_rect,
                    "sheet-confirm",
                    &action,
                    Style::Plain,
                    !*busy,
                    focus == Some(1),
                    palette,
                ) {
                    self.store.confirm_trash();
                }
            }
            _ => {
                let button = Rect::from_min_size(
                    pos2(rect.right() - 24.0 - 80.0, buttons_y),
                    vec2(80.0, 28.0),
                );
                if push_button(
                    ui,
                    button,
                    "sheet-ok",
                    "OK",
                    Style::Plain,
                    true,
                    focus == Some(0),
                    palette,
                ) {
                    self.close_sheet();
                }
            }
        }
    }

    #[allow(clippy::type_complexity)]
    fn sheet_copy(&self, sheet: &Sheet) -> (String, String, Option<String>, Vec<(String, String)>) {
        match sheet {
            Sheet::Confirm { photos, .. } if self.store.is_library() => (
                format!("Flytte {} til Nylig slettet?", images(photos.len())),
                "De fjernes fra Bilder på alle enhetene dine med iCloud-bilder, også for andre hvis de ligger i et delt bibliotek. De kan hentes tilbake i 30 dager. macOS ber deg bekrefte én gang til.".to_owned(),
                Some("Flytt til Nylig slettet".to_owned()),
                Vec::new(),
            ),
            Sheet::Confirm { photos, bytes: size, .. } => (
                format!("Flytte {} til papirkurven?", images(photos.len())),
                format!("Dette frigjør {} når papirkurven tømmes.", bytes(*size)),
                Some("Flytt til papirkurven".to_owned()),
                Vec::new(),
            ),
            Sheet::Failures { failed, moved } => {
                let also = if *moved > 0 {
                    format!(" {} ble flyttet.", images(*moved))
                } else {
                    String::new()
                };
                (
                    format!("{} kunne ikke flyttes", images(failed.len())),
                    format!("De ligger fortsatt i Til sletting.{also}"),
                    None,
                    failed.clone(),
                )
            }
        }
    }
}

/// Old workers may be waiting on iCloud for a while, so they are not waited for here.
fn discard_images(images: Images) {
    std::thread::spawn(move || drop(images));
}

fn access_help(access: Status) -> &'static str {
    if access == Status::Restricted {
        "Tilgangen til Bilder er begrenset på denne Mac-en, for eksempel av en administrator."
    } else {
        "Bildesveip har ikke tilgang til Bilder. Slå det på i Systeminnstillinger, under Personvern og sikkerhet."
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{scan::FolderState, store::tests::folder};
    use eframe::egui::{ColorImage, Theme};
    use egui_kittest::Harness;
    use std::cell::RefCell;

    fn scene(width: usize, height: usize, top: [u8; 3], bottom: [u8; 3]) -> ColorImage {
        let mut pixels = Vec::with_capacity(width * height);
        for y in 0..height {
            let t = y as f32 / height as f32;
            let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t) as u8;
            let color = Color32::from_rgb(
                mix(top[0], bottom[0]),
                mix(top[1], bottom[1]),
                mix(top[2], bottom[2]),
            );
            pixels.extend(std::iter::repeat_n(color, width));
        }
        ColorImage::new([width, height], pixels)
    }

    fn test_app(ctx: &Context, state: FolderState) -> App {
        let mut store = Store::default();
        store.open(state.folder.clone());
        store.outbox.clear();
        let count = state.photos.len();
        store.on_opened(1, Ok(state), None);
        let mut images = Images::idle();
        for photo in 0..count {
            let palette = [
                ([255, 197, 107], [18, 78, 96]),
                ([75, 163, 255], [21, 88, 214]),
                ([52, 199, 89], [0, 60, 40]),
            ][photo % 3];
            let (width, height) = if photo % 2 == 0 {
                (1000, 640)
            } else {
                (640, 1000)
            };
            images.insert_for_test(ctx, photo, scene(width, height, palette.0, palette.1));
        }
        App {
            store,
            images,
            sweep: Sweep::default(),
            grid: Grid::default(),
            settings: SystemSettings {
                accent: Color32::from_rgb(0x0a, 0x84, 0xff),
                reduce_motion: false,
            },
            backend: None,
            menu: None,
            cache: PathBuf::new(),
            sheet_focus: None,
            last_action: None,
            title: String::new(),
            wants_folder_dialog: false,
            quit_when_idle: false,
            started: Instant::now(),
        }
    }

    fn names(count: usize) -> Vec<String> {
        (0..count).map(|i| format!("IMG_{i:04}.HEIC")).collect()
    }

    fn sample(ctx: &Context, count: usize, marks: &[Decision]) -> App {
        let names = names(count);
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let mut state = folder(&refs, marks);
        state.folder_name = "Ferie 2026".into();
        test_app(ctx, state)
    }

    fn sample_library(ctx: &Context) -> App {
        let names = names(6);
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let mut state = folder(&refs, &[Decision::Discard; 6]);
        state.source = "library";
        state.folder = library::LIBRARY_PATH.into();
        state.folder_name = "Bilder".into();
        for (index, photo) in state.photos.iter_mut().enumerate() {
            photo.path = format!("bilder:asset:{index}");
            photo.modified = 1_780_000_000_000 + index as u64 * 86_400_000;
            photo.size = 0;
        }
        test_app(ctx, state)
    }

    fn press(app: &mut App, ctx: &Context, action: Action) {
        app.sweep.acted = false;
        app.last_action = None;
        app.run(ctx, action, 100.0);
    }

    #[test]
    fn keys_and_menu_for_the_same_press_count_once() {
        let ctx = Context::default();
        let mut app = sample(&ctx, 3, &[]);
        app.run_at(&ctx, Action::Keep, 1.0, 1.0);
        app.sweep.acted = false;
        app.run_at(&ctx, Action::Keep, 1.05, 1.05);
        assert_eq!(app.store.counts().1, 1);
        app.sweep.acted = false;
        app.run_at(&ctx, Action::Keep, 1.3, 1.3);
        assert_eq!(app.store.counts().1, 2);
    }

    #[test]
    fn only_one_photo_is_sorted_per_frame() {
        let ctx = Context::default();
        let mut app = sample(&ctx, 3, &[]);
        app.choose(true, 1.0);
        app.choose(true, 1.0);
        app.skip(1.0);
        assert_eq!(app.store.counts().1, 1);
    }

    #[test]
    fn a_photo_that_has_not_loaded_cannot_be_decided() {
        let ctx = Context::default();
        let mut app = sample(&ctx, 2, &[]);
        app.images = Images::idle();
        app.choose(true, 1.0);
        assert_eq!(app.store.counts().1, 0);
    }

    #[test]
    fn a_library_photo_that_failed_to_load_stays_undecided() {
        let ctx = Context::default();
        let mut app = sample_library(&ctx);
        if let Some(folder) = app.store.folder.as_mut() {
            folder.photos[0].decision = Decision::Undecided;
        }
        app.store.queue.push_front(0);
        app.images.fail_preview_for_test(0);
        assert!(!app.can_decide());
        app.choose(false, 1.0);
        assert_eq!(app.store.current(), Some(0));
    }

    #[test]
    fn undoing_during_the_exit_animation_removes_the_leaving_copy() {
        let ctx = Context::default();
        let mut app = sample(&ctx, 2, &[]);
        app.choose(true, 1.0);
        assert_eq!(app.sweep.leaving.len(), 1);
        press(&mut app, &ctx, Action::Undo);
        assert!(app.sweep.leaving.is_empty());
    }

    #[test]
    fn return_never_confirms_without_keyboard_focus() {
        let ctx = Context::default();
        let mut app = sample(&ctx, 2, &[Decision::Discard, Decision::Discard]);
        app.store.set_view(View::Review);
        app.store.ask_trash();
        app.sheet_key(Key::Enter, Modifiers::NONE, false);
        assert!(app.store.outbox.is_empty());
        app.sheet_key(Key::Tab, Modifiers::NONE, false);
        app.sheet_key(Key::Tab, Modifiers::NONE, false);
        app.sheet_key(Key::Enter, Modifiers::NONE, false);
        assert_eq!(app.store.outbox.len(), 1);
    }

    #[test]
    fn a_new_confirmation_does_not_inherit_keyboard_focus() {
        let ctx = Context::default();
        let mut app = sample(&ctx, 2, &[Decision::Discard, Decision::Discard]);
        app.store.set_view(View::Review);
        app.store.ask_trash();
        app.sheet_focus = Some(1);
        app.store.confirm_trash();
        app.on_trashed(Ok(TrashResult {
            trashed: vec!["IMG_0000.HEIC".into()],
            failed: vec![],
            bytes_freed: 1,
            state_error: None,
        }));
        assert_eq!(app.sheet_focus, None);
    }

    #[test]
    fn quitting_waits_while_a_system_dialog_may_be_up() {
        let ctx = Context::default();
        let mut app = sample(&ctx, 2, &[Decision::Discard, Decision::Discard]);
        app.store.set_view(View::Review);
        app.store.ask_trash();
        assert!(!app.waiting_on_system());
        app.store.confirm_trash();
        assert!(app.waiting_on_system());
    }

    #[test]
    fn a_new_folder_never_keeps_images_from_the_last_one() {
        let ctx = Context::default();
        let mut app = sample(&ctx, 2, &[]);
        assert!(app.can_decide());
        app.install_new_folder(&ctx);
        assert!(app.images.preview(0).is_none());
        assert!(!app.can_decide());
    }

    #[test]
    fn escape_cancels_the_confirmation() {
        let ctx = Context::default();
        let mut app = sample(&ctx, 1, &[Decision::Discard]);
        app.store.set_view(View::Review);
        app.store.ask_trash();
        app.sheet_key(Key::Escape, Modifiers::NONE, false);
        assert!(app.store.sheet.is_none());
        assert!(app.store.outbox.is_empty());
    }

    #[test]
    fn nothing_from_the_menu_reaches_the_app_behind_a_dialog() {
        let ctx = Context::default();
        let mut app = sample(&ctx, 2, &[Decision::Discard]);
        app.store.set_view(View::Review);
        app.store.ask_trash();
        press(&mut app, &ctx, Action::OpenLibrary);
        assert!(app.store.outbox.is_empty());
    }

    /// Writes PNGs for looking at the design: `SNAPSHOT_DIR=dir cargo test render -- --ignored`.
    #[test]
    #[ignore = "writes images, run by hand"]
    fn render_states() {
        let dir = PathBuf::from(std::env::var("SNAPSHOT_DIR").expect("SNAPSHOT_DIR"));
        std::fs::create_dir_all(&dir).unwrap();
        type Setup = fn(&Context) -> App;
        let states: [(&str, Theme, Setup); 8] = [
            ("tom", Theme::Dark, |ctx| {
                let mut app = sample(ctx, 1, &[]);
                app.store.folder = None;
                app
            }),
            ("sveip-mork", Theme::Dark, |ctx| {
                let mut app = sample(ctx, 12, &[Decision::Keep, Decision::Discard]);
                app.sweep.drag = -150.0;
                app
            }),
            ("sveip-lys", Theme::Light, |ctx| {
                sample(ctx, 12, &[Decision::Keep])
            }),
            ("rutenett-mork", Theme::Dark, |ctx| {
                let mut app = sample(ctx, 30, &[Decision::Discard; 30]);
                app.store.set_view(View::Review);
                app.store.selection = [2, 3].into();
                app.store.cursor = Some(3);
                app
            }),
            ("rutenett-lys", Theme::Light, |ctx| {
                let mut app = sample(ctx, 9, &[Decision::Keep; 9]);
                app.store.set_view(View::Kept);
                app.store.status = Some("2 bilder lagt tilbake i Sveip.".to_owned());
                app
            }),
            ("bekreft-papirkurv", Theme::Dark, |ctx| {
                let mut app = sample(ctx, 8, &[Decision::Discard; 8]);
                app.store.set_view(View::Review);
                app.store.ask_trash();
                app
            }),
            ("bekreft-bilder", Theme::Light, |ctx| {
                let mut app = sample_library(ctx);
                app.store.set_view(View::Review);
                app.store.ask_trash();
                app
            }),
            ("feilet-bibliotek", Theme::Dark, |ctx| {
                let mut app = sample_library(ctx);
                if let Some(folder) = app.store.folder.as_mut() {
                    folder.photos[0].decision = Decision::Undecided;
                }
                app.store.queue.push_front(0);
                app.images.fail_preview_for_test(0);
                app.store.library_access = Some(Status::Denied);
                app
            }),
        ];
        for (name, theme, setup) in states {
            let holder: RefCell<Option<App>> = RefCell::new(None);
            let mut harness = Harness::builder()
                .with_size(vec2(1180.0, 780.0))
                .with_pixels_per_point(2.0)
                .build_ui(|ui| {
                    let mut slot = holder.borrow_mut();
                    let app = slot.get_or_insert_with(|| {
                        crate::install_system_font(ui.ctx());
                        ui.ctx().set_theme(theme);
                        setup(ui.ctx())
                    });
                    app.tick(ui.ctx());
                    app.show(ui);
                });
            for _ in 0..6 {
                harness.step();
            }
            harness
                .render()
                .unwrap()
                .save(dir.join(format!("{name}.png")))
                .unwrap();
        }
    }
}
