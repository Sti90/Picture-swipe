//! Colours and hand-drawn controls in the style of macOS.
use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Painter, Pos2, Rect, Response, Sense, Stroke,
    Theme, Ui, Vec2, WidgetInfo, vec2,
};

#[derive(Clone, Copy)]
pub struct Palette {
    pub background: Color32,
    pub sidebar: Color32,
    pub label: Color32,
    pub secondary: Color32,
    pub tertiary: Color32,
    pub separator: Color32,
    pub fill: Color32,
    pub accent: Color32,
    pub error: Color32,
}

impl Palette {
    pub fn new(theme: Theme, accent: Color32) -> Self {
        match theme {
            Theme::Dark => Self {
                background: Color32::from_rgb(0x1c, 0x1c, 0x1e),
                sidebar: Color32::from_rgb(0x2a, 0x2a, 0x2c),
                label: Color32::from_white_alpha(225),
                secondary: Color32::from_white_alpha(140),
                tertiary: Color32::from_white_alpha(70),
                separator: Color32::from_white_alpha(26),
                fill: Color32::from_white_alpha(22),
                accent,
                error: Color32::from_rgb(0xff, 0x6b, 0x6e),
            },
            Theme::Light => Self {
                background: Color32::from_rgb(0xf5, 0xf5, 0xf7),
                sidebar: Color32::from_rgb(0xe8, 0xe8, 0xeb),
                label: Color32::from_black_alpha(218),
                secondary: Color32::from_black_alpha(130),
                tertiary: Color32::from_black_alpha(66),
                separator: Color32::from_black_alpha(26),
                fill: Color32::from_black_alpha(14),
                accent,
                error: Color32::from_rgb(0xc4, 0x1e, 0x22),
            },
        }
    }
}

pub fn font(size: f32) -> FontId {
    FontId::proportional(size)
}

pub fn text(
    painter: &Painter,
    at: Pos2,
    align: Align2,
    value: impl ToString,
    size: f32,
    color: Color32,
) {
    painter.text(at, align, value, font(size), color);
}

pub fn text_width(ui: &Ui, value: &str, size: f32) -> f32 {
    ui.painter()
        .layout_no_wrap(value.to_owned(), font(size), Color32::WHITE)
        .size()
        .x
}

/// Shortens with an ellipsis so a long file name can't run into its neighbours.
pub fn fit_text(ui: &Ui, value: &str, size: f32, width: f32) -> String {
    if text_width(ui, value, size) <= width {
        return value.to_owned();
    }
    let characters: Vec<char> = value.chars().collect();
    let (mut low, mut high) = (0, characters.len());
    while low < high {
        let middle = (low + high).div_ceil(2);
        let candidate: String = characters[..middle].iter().chain(['…'].iter()).collect();
        if text_width(ui, &candidate, size) <= width {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    characters[..low].iter().chain(['…'].iter()).collect()
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Sweep,
    Trash,
    Check,
    Folder,
    Library,
    Undo,
    Reveal,
    Close,
    Warning,
    Photo,
}

pub fn icon(painter: &Painter, c: Pos2, kind: Icon, color: Color32) {
    let stroke = Stroke::new(1.5, color);
    let line = |points: &[(f32, f32)]| {
        let points: Vec<Pos2> = points.iter().map(|(x, y)| c + vec2(*x, *y)).collect();
        painter.add(egui::Shape::line(points, stroke));
    };
    let frame = |rect: Rect, radius: u8| {
        painter.rect_stroke(
            rect,
            CornerRadius::same(radius),
            stroke,
            egui::StrokeKind::Middle,
        );
    };
    match kind {
        Icon::Sweep => {
            frame(
                Rect::from_center_size(c - vec2(2.0, 0.0), vec2(10.0, 13.0)),
                2,
            );
            line(&[(5.0, -4.0), (8.0, 0.0), (5.0, 4.0)]);
        }
        Icon::Trash => {
            line(&[(-6.0, -4.5), (6.0, -4.5)]);
            line(&[(-4.5, -4.5), (-3.5, 6.0), (3.5, 6.0), (4.5, -4.5)]);
            line(&[(-2.0, -6.5), (2.0, -6.5)]);
        }
        Icon::Check => line(&[(-5.0, 0.0), (-1.5, 4.0), (5.5, -5.0)]),
        Icon::Folder => {
            line(&[
                (-7.0, 5.0),
                (-7.0, -5.0),
                (-2.5, -5.0),
                (-0.5, -3.0),
                (7.0, -3.0),
                (7.0, 5.0),
                (-7.0, 5.0),
            ]);
        }
        Icon::Library | Icon::Photo => {
            frame(Rect::from_center_size(c, vec2(14.0, 11.0)), 2);
            line(&[
                (-6.0, 4.0),
                (-2.0, -0.5),
                (1.0, 2.5),
                (3.0, 0.5),
                (6.0, 4.0),
            ]);
            painter.circle_filled(c + vec2(3.2, -2.6), 1.2, color);
        }
        Icon::Undo => {
            line(&[(-3.0, -6.0), (-6.5, -2.5), (-3.0, 1.0)]);
            line(&[
                (-6.5, -2.5),
                (2.0, -2.5),
                (6.0, 0.5),
                (6.0, 3.0),
                (2.0, 6.0),
                (-3.0, 6.0),
            ]);
        }
        Icon::Reveal => {
            painter.circle_stroke(c - vec2(1.5, 1.5), 4.5, stroke);
            line(&[(2.0, 2.0), (6.5, 6.5)]);
        }
        Icon::Close => {
            line(&[(-4.0, -4.0), (4.0, 4.0)]);
            line(&[(4.0, -4.0), (-4.0, 4.0)]);
        }
        Icon::Warning => {
            line(&[(0.0, -6.0), (7.0, 6.0), (-7.0, 6.0), (0.0, -6.0)]);
            line(&[(0.0, -2.0), (0.0, 2.0)]);
            painter.circle_filled(c + vec2(0.0, 4.2), 0.9, color);
        }
    }
}

fn named(response: Response, enabled: bool, label: &str) -> Response {
    let label = label.to_owned();
    response.widget_info(move || WidgetInfo::labeled(egui::Role::Button, enabled, label.clone()));
    response
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Style {
    Plain,
    Prominent,
}

pub fn button_width(ui: &Ui, label: &str) -> f32 {
    text_width(ui, label, 13.0) + 26.0
}

/// A standard push button. `focused` draws the ring for keyboard focus.
#[allow(clippy::too_many_arguments)]
pub fn push_button(
    ui: &mut Ui,
    rect: Rect,
    id: &str,
    label: &str,
    style: Style,
    enabled: bool,
    focused: bool,
    palette: Palette,
) -> bool {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let response = named(
        ui.interact(rect, ui.scope_id().with(id), sense),
        enabled,
        label,
    );
    let painter = ui.painter();
    let pressed = response.is_pointer_button_down_on();
    let (fill, color) = match (style, enabled) {
        (Style::Prominent, true) => (
            if pressed {
                palette.accent.gamma_multiply(0.8)
            } else {
                palette.accent
            },
            Color32::WHITE,
        ),
        (Style::Plain, true) => (
            if pressed {
                palette.fill.gamma_multiply(2.0)
            } else {
                palette.fill.gamma_multiply(1.4)
            },
            palette.label,
        ),
        (_, false) => (palette.fill, palette.tertiary),
    };
    painter.rect_filled(rect, CornerRadius::same(7), fill);
    if focused {
        painter.rect_stroke(
            rect.expand(2.0),
            CornerRadius::same(9),
            Stroke::new(2.0, palette.accent.gamma_multiply(0.6)),
            egui::StrokeKind::Middle,
        );
    }
    text(
        painter,
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        13.0,
        color,
    );
    response.clicked()
}

/// A borderless toolbar button, as the guidelines ask for.
pub fn tool_button(
    ui: &mut Ui,
    rect: Rect,
    id: &str,
    kind: Icon,
    label: &str,
    enabled: bool,
    palette: Palette,
) -> bool {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let response = named(
        ui.interact(rect, ui.scope_id().with(id), sense),
        enabled,
        label,
    );
    let painter = ui.painter();
    if enabled && response.hovered() {
        painter.rect_filled(rect, CornerRadius::same(6), palette.fill);
    }
    icon(
        painter,
        rect.center(),
        kind,
        if enabled {
            palette.secondary
        } else {
            palette.tertiary
        },
    );
    let response = if enabled {
        response.on_hover_text(label.to_owned())
    } else {
        response
    };
    response.clicked()
}

pub struct RowContent<'a> {
    pub kind: Icon,
    pub label: &'a str,
    pub count: Option<String>,
    pub selected: bool,
}

/// A sidebar row.
pub fn sidebar_row(
    ui: &mut Ui,
    rect: Rect,
    id: &str,
    row: RowContent<'_>,
    enabled: bool,
    palette: Palette,
) -> bool {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let response = named(
        ui.interact(rect, ui.scope_id().with(id), sense),
        enabled,
        row.label,
    );
    let painter = ui.painter();
    if row.selected {
        painter.rect_filled(
            rect,
            CornerRadius::same(6),
            palette.accent.gamma_multiply(0.28),
        );
    } else if enabled && response.hovered() {
        painter.rect_filled(rect, CornerRadius::same(6), palette.fill);
    }
    let color = if enabled {
        palette.label
    } else {
        palette.tertiary
    };
    icon(
        painter,
        rect.left_center() + vec2(16.0, 0.0),
        row.kind,
        if row.selected {
            palette.accent
        } else {
            palette.secondary
        },
    );
    text(
        painter,
        rect.left_center() + vec2(33.0, 0.0),
        Align2::LEFT_CENTER,
        row.label,
        13.0,
        color,
    );
    if let Some(count) = row.count {
        text(
            painter,
            rect.right_center() - vec2(10.0, 0.0),
            Align2::RIGHT_CENTER,
            count,
            12.0,
            palette.secondary,
        );
    }
    response.clicked()
}

pub const GLASS_FILL: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 140);

/// One half of the glass keep/discard control over a photo.
pub fn glass_button(
    ui: &mut Ui,
    rect: Rect,
    id: &str,
    kind: Icon,
    label: &str,
    enabled: bool,
) -> bool {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let response = named(
        ui.interact(rect, ui.scope_id().with(id), sense),
        enabled,
        label,
    );
    let painter = ui.painter();
    if response.hovered() && enabled {
        painter.rect_filled(
            rect.shrink(3.0),
            CornerRadius::same(17),
            Color32::from_white_alpha(22),
        );
    }
    let color = if enabled {
        Color32::WHITE
    } else {
        Color32::from_white_alpha(90)
    };
    let width = text_width(ui, label, 13.0);
    let start = rect.center().x - (width + 24.0) / 2.0;
    icon(
        ui.painter(),
        Pos2::new(start + 8.0, rect.center().y),
        kind,
        color,
    );
    text(
        ui.painter(),
        Pos2::new(start + 24.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        13.0,
        color,
    );
    response.clicked()
}

pub fn glass_pill(painter: &Painter, rect: Rect) {
    painter.rect_filled(rect, CornerRadius::same(20), GLASS_FILL);
    painter.rect_stroke(
        rect,
        CornerRadius::same(20),
        Stroke::new(1.0, Color32::from_white_alpha(38)),
        egui::StrokeKind::Inside,
    );
}

pub fn glass_label(painter: &Painter, center: Pos2, label: &str, opacity: f32) {
    let rect = Rect::from_center_size(center, Vec2::new(92.0, 40.0));
    painter.rect_filled(
        rect,
        CornerRadius::same(20),
        GLASS_FILL.gamma_multiply(opacity),
    );
    text(
        painter,
        center,
        Align2::CENTER_CENTER,
        label,
        14.0,
        Color32::WHITE.gamma_multiply(opacity),
    );
}
