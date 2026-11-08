//! The "Til sletting" and "Beholdt" views: a grid that can hold tens of thousands of photos.
use eframe::egui::{
    self, Align2, Color32, CornerRadius, Modifiers, Rect, ScrollArea, Sense, Stroke, Ui, UiBuilder,
    Vec2, vec2,
};

use crate::{
    app::App,
    format::photo_title,
    store::View,
    widgets::{self, Icon, Palette},
};

const TARGET_CELL: f32 = 168.0;
const GAP: f32 = 10.0;
const PADDING: f32 = 16.0;
const CAPTION: f32 = 26.0;
const OVERSCAN_ROWS: usize = 2;

#[derive(Clone, Copy)]
struct Layout {
    columns: usize,
    cell: f32,
}

impl Layout {
    fn new(width: f32) -> Self {
        let inner = (width - PADDING * 2.0).max(TARGET_CELL);
        let columns = (((inner + GAP) / (TARGET_CELL + GAP)).floor() as usize).max(1);
        let cell = (inner - GAP * (columns - 1) as f32) / columns as f32;
        Self { columns, cell }
    }
    fn row_height(self) -> f32 {
        self.cell + CAPTION + GAP
    }
    fn content_height(self, photos: usize) -> f32 {
        let rows = photos.div_ceil(self.columns);
        if rows == 0 {
            0.0
        } else {
            rows as f32 * self.row_height() - GAP + PADDING * 2.0
        }
    }
    fn top_of(self, index: usize) -> f32 {
        PADDING + (index / self.columns) as f32 * self.row_height()
    }
}

#[derive(Default)]
pub struct Grid {
    pub columns: usize,
    cell: f32,
    scroll_to: Option<f32>,
    offset: f32,
    viewport: f32,
    wanted: Vec<usize>,
}

impl Grid {
    pub fn visible(&self) -> &[usize] {
        &self.wanted
    }
}

impl App {
    pub fn show_grid(&mut self, ui: &mut Ui, area: Rect, palette: Palette) {
        let view = self.store.view;
        let list = self.store.list(view).to_vec();
        if list.is_empty() {
            self.grid.wanted.clear();
            let (title, body) = if view == View::Review {
                (
                    "Ingen bilder til sletting",
                    "Bilder du kaster i Sveip, havner her først.",
                )
            } else {
                (
                    "Ingen beholdte bilder",
                    "Bilder du beholder i Sveip, vises her.",
                )
            };
            let center = area.center();
            widgets::text(
                ui.painter(),
                center - vec2(0.0, 12.0),
                Align2::CENTER_CENTER,
                title,
                17.0,
                palette.label,
            );
            widgets::text(
                ui.painter(),
                center + vec2(0.0, 14.0),
                Align2::CENTER_CENTER,
                body,
                13.0,
                palette.secondary,
            );
            return;
        }
        let layout = Layout::new(area.width() - 10.0);
        self.grid.columns = layout.columns;
        self.grid.cell = layout.cell;
        let modal = self.store.locked();
        let mut scroll = ScrollArea::vertical()
            .auto_shrink([false, false])
            .id_salt(("grid", view as u8));
        if modal {
            scroll = scroll.scroll_source(egui::scroll_area::ScrollSource::NONE);
        }
        if let Some(offset) = self.grid.scroll_to.take() {
            scroll = scroll.vertical_scroll_offset(offset);
        }
        let mut child = ui.new_child(UiBuilder::new().max_rect(area));
        let output = scroll.show_viewport(&mut child, |ui, viewport| {
            self.grid.offset = viewport.top();
            let origin = ui.max_rect().min;
            let (_, content) = ui.allocate_space(vec2(
                ui.available_width(),
                layout.content_height(list.len()),
            ));
            if ui
                .interact(content, ui.scope_id().with("background"), Sense::click())
                .clicked()
                && !modal
            {
                self.store.clear_selection();
            }
            let first =
                ((viewport.top() - PADDING).max(0.0) / layout.row_height()).floor() as usize;
            let last = (viewport.bottom() / layout.row_height()).ceil() as usize;
            let start = first.saturating_sub(OVERSCAN_ROWS) * layout.columns;
            let end = ((last + OVERSCAN_ROWS + 1) * layout.columns).min(list.len());
            self.grid.wanted = list[start.min(end)..end].to_vec();
            for index in start..end {
                let column = index % layout.columns;
                let x = PADDING + column as f32 * (layout.cell + GAP);
                let cell = Rect::from_min_size(
                    origin + vec2(x, layout.top_of(index)),
                    Vec2::new(layout.cell, layout.cell + CAPTION),
                );
                self.cell(ui, cell, layout, list[index], &list, palette, modal);
            }
        });
        self.grid.viewport = output.inner_rect.height();
    }

    #[allow(clippy::too_many_arguments)]
    fn cell(
        &mut self,
        ui: &mut Ui,
        cell: Rect,
        layout: Layout,
        photo: usize,
        list: &[usize],
        palette: Palette,
        modal: bool,
    ) {
        let box_rect = Rect::from_min_size(cell.min, Vec2::splat(layout.cell));
        let title = photo_title(&self.store.photos()[photo]);
        let response = ui.interact(cell, ui.scope_id().with(("cell", photo)), Sense::click());
        let selected = self.store.selection.contains(&photo);
        let painter = ui.painter().clone();
        painter.rect_filled(box_rect, CornerRadius::same(6), palette.fill);
        if let Some(texture) = self.images.thumbnail(photo) {
            let size = texture.size_vec2();
            let scale = (box_rect.width() / size.x).min(box_rect.height() / size.y);
            let image = Rect::from_center_size(box_rect.center(), size * scale);
            let uv = Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0));
            painter.image(texture.id(), image, uv, Color32::WHITE);
        } else if self.images.thumbnail_failed(photo) {
            widgets::icon(&painter, box_rect.center(), Icon::Photo, palette.tertiary);
        }
        if selected {
            painter.rect_stroke(
                box_rect.expand(1.5),
                CornerRadius::same(8),
                Stroke::new(3.0, palette.accent),
                egui::StrokeKind::Middle,
            );
        }
        let caption = widgets::fit_text(ui, &title, 11.0, layout.cell - 8.0);
        let center = egui::pos2(cell.center().x, box_rect.bottom() + CAPTION / 2.0);
        widgets::text(
            &painter,
            center,
            Align2::CENTER_CENTER,
            caption,
            11.0,
            if selected {
                palette.label
            } else {
                palette.secondary
            },
        );
        if response.clicked() && !modal {
            let modifiers = ui.input(|i| i.modifiers);
            self.click_cell(photo, list, modifiers);
        }
    }

    fn click_cell(&mut self, photo: usize, list: &[usize], modifiers: Modifiers) {
        if modifiers.shift
            && let Some(anchor) = self.store.anchor
        {
            self.select_range(anchor, photo, list);
        } else if modifiers.command {
            if !self.store.selection.remove(&photo) {
                self.store.selection.insert(photo);
            }
            self.store.anchor = Some(photo);
        } else {
            self.store.selection = [photo].into();
            self.store.anchor = Some(photo);
        }
        self.store.cursor = Some(photo);
    }

    fn select_range(&mut self, from: usize, to: usize, list: &[usize]) {
        let position = |photo| list.iter().position(|p| *p == photo);
        if let (Some(a), Some(b)) = (position(from), position(to)) {
            self.store.selection = list[a.min(b)..=a.max(b)].iter().copied().collect();
        }
    }

    pub fn move_cursor(&mut self, dx: isize, dy: isize, extend: bool) {
        let view = self.store.view;
        let list = self.store.list(view).to_vec();
        if list.is_empty() {
            return;
        }
        let columns = self.grid.columns.max(1) as isize;
        let current = self
            .store
            .cursor
            .and_then(|c| list.iter().position(|p| *p == c));
        let index = match current {
            None => 0,
            Some(current) => {
                let next = current as isize + if dy != 0 { dy * columns } else { dx };
                if next < 0 || next >= list.len() as isize {
                    return;
                }
                next as usize
            }
        };
        let photo = list[index];
        match self.store.anchor.filter(|_| extend) {
            Some(anchor) => self.select_range(anchor, photo, &list),
            None => {
                self.store.selection = [photo].into();
                self.store.anchor = Some(photo);
            }
        }
        self.store.cursor = Some(photo);
        self.scroll_into_view(index);
    }

    fn scroll_into_view(&mut self, index: usize) {
        let layout = Layout {
            columns: self.grid.columns.max(1),
            cell: self.grid.cell,
        };
        let top = layout.top_of(index);
        let bottom = top + layout.cell + CAPTION;
        if top < self.grid.offset + PADDING {
            self.grid.scroll_to = Some(top - PADDING);
        } else if bottom > self.grid.offset + self.grid.viewport - PADDING {
            self.grid.scroll_to = Some(bottom - self.grid.viewport + PADDING);
        }
    }

    pub fn select_all(&mut self) {
        let view = self.store.view;
        let list = self.store.list(view).to_vec();
        self.store.selection = list.iter().copied().collect();
        self.store.anchor = list.first().copied();
        self.store.cursor = list.last().copied();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_grid_fits_the_width_and_stays_within_it() {
        let layout = Layout::new(1000.0);
        let used =
            layout.columns as f32 * layout.cell + (layout.columns - 1) as f32 * GAP + PADDING * 2.0;
        assert!((used - 1000.0).abs() < 0.5);
        assert_eq!(Layout::new(100.0).columns, 1);
    }

    #[test]
    fn content_height_is_exact_for_partial_rows() {
        let layout = Layout {
            columns: 4,
            cell: 100.0,
        };
        assert_eq!(layout.content_height(0), 0.0);
        assert_eq!(layout.content_height(1), layout.content_height(4));
        assert!(layout.content_height(5) > layout.content_height(4));
    }
}
