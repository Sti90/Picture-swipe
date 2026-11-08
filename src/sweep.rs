//! Sorting one photo at a time, with the keyboard, the mouse or a two-finger swipe.
use eframe::{
    egui::{
        self, Align2, Color32, Context, Event, Mesh, MouseWheelSource, MouseWheelUnit, Painter,
        Pos2, Rect, Sense, Stroke, TextureHandle, TouchPhase, Ui, Vec2, emath::Rot2, pos2, vec2,
    },
    epaint::Shape,
};

use crate::{
    app::App,
    format::{bytes, images, photo_title, when},
    store::Motion,
    widgets::{self, Icon, Palette, Style, glass_button, glass_label, glass_pill, push_button},
};

const LEAVE_SECONDS: f64 = 0.22;
const SPRING_SECONDS: f64 = 0.36;
const ARRIVE_SECONDS: f64 = 0.16;
const WHEEL_QUIET: f64 = 0.12;
/// How long a source without gesture phases must be still before it counts as a new swipe.
const NEW_GESTURE_QUIET: f64 = 0.18;
const SHOW_SPINNER_AFTER: f64 = 0.4;
pub const FOOTER: f32 = 76.0;
const MESSAGE_ROW: f32 = 34.0;

pub struct Leaving {
    pub photo: usize,
    texture: Option<TextureHandle>,
    from: f32,
    direction: Direction,
    start: f64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Direction {
    Left,
    Right,
    Down,
}

#[derive(Clone, Copy)]
enum Arrival {
    Soft,
    From(f32),
}

#[derive(Default)]
struct Wheel {
    dx: f32,
    active: bool,
    locked: bool,
    last: f64,
}

pub struct Sweep {
    pub drag: f32,
    wheel: Wheel,
    settle: Option<(f32, f64)>,
    pub leaving: Vec<Leaving>,
    arrival: (Arrival, f64),
    shown_since: f64,
    /// One photo per frame, however many input sources fired.
    pub acted: bool,
    shown: Option<usize>,
}

impl Default for Sweep {
    fn default() -> Self {
        Self {
            drag: 0.0,
            wheel: Wheel::default(),
            settle: None,
            leaving: Vec::new(),
            arrival: (Arrival::Soft, f64::NEG_INFINITY),
            shown_since: 0.0,
            acted: false,
            shown: None,
        }
    }
}

impl Sweep {
    fn offset(&self) -> f32 {
        self.drag + self.wheel.dx
    }

    fn reset(&mut self) {
        self.drag = 0.0;
        self.wheel.dx = 0.0;
        self.wheel.active = false;
        self.settle = None;
    }

    fn spring_back(&mut self, from: f32, now: f64) {
        if from != 0.0 {
            self.settle = Some((from, now));
        }
    }
}

impl App {
    /// A photo can't be judged until it is visible, which may take an iCloud download.
    /// A folder photo that fails to load may still be sorted, as its name is shown.
    pub fn can_decide(&self) -> bool {
        self.store.current().is_some_and(|photo| {
            self.images.preview(photo).is_some()
                || (!self.store.is_library() && self.images.preview_failed(photo))
        })
    }

    pub fn choose(&mut self, keep: bool, now: f64) {
        if self.sweep.acted || !self.can_decide() {
            return;
        }
        let from = self.sweep.offset();
        let Some(photo) = self.store.decide(keep) else {
            return;
        };
        self.sweep.acted = true;
        let direction = if keep {
            Direction::Right
        } else {
            Direction::Left
        };
        self.leave(photo, from, direction, now);
    }

    pub fn skip(&mut self, now: f64) {
        if self.sweep.acted {
            return;
        }
        let from = self.sweep.offset();
        if let Some(photo) = self.store.skip() {
            self.sweep.acted = true;
            self.leave(photo, from, Direction::Down, now);
        }
    }

    fn leave(&mut self, photo: usize, from: f32, direction: Direction, now: f64) {
        if !self.settings.reduce_motion {
            self.sweep.leaving.push(Leaving {
                photo,
                texture: self.images.preview(photo).cloned(),
                from,
                direction,
                start: now,
            });
        }
        self.sweep.reset();
        self.sweep.arrival = (Arrival::Soft, now);
        self.sweep.shown_since = now;
    }

    pub fn undo_sweep_animation(&mut self, restored: &[usize], width: f32, now: f64) {
        self.sweep
            .leaving
            .retain(|card| !restored.contains(&card.photo));
        if let Some(motion) = self.store.motion.take() {
            let side = if motion == Motion::InFromRight {
                1.0
            } else {
                -1.0
            };
            self.sweep.reset();
            self.sweep.arrival = (Arrival::From(side * width * 0.75), now);
            self.sweep.shown_since = now;
        }
    }

    fn read_trackpad(&mut self, ctx: &Context, threshold: f32, now: f64, can_decide: bool) {
        let events = ctx.input(|i| i.events.clone());
        for event in events {
            let Event::MouseWheel {
                unit: MouseWheelUnit::Point,
                delta,
                phase,
                source,
                ..
            } = event
            else {
                continue;
            };
            // Inertia is the trackpad coasting after the fingers lifted, never a new choice.
            if source == MouseWheelSource::Momentum {
                continue;
            }
            // A trackpad reports when a gesture starts. Pausing mid-gesture is not a new one.
            let new_gesture = phase == TouchPhase::Start
                || (source != MouseWheelSource::Trackpad
                    && now - self.sweep.wheel.last > NEW_GESTURE_QUIET);
            if new_gesture {
                self.sweep.wheel.locked = false;
            }
            self.sweep.wheel.last = now;
            if self.sweep.wheel.locked || !can_decide {
                continue;
            }
            if matches!(phase, TouchPhase::End | TouchPhase::Cancel) {
                let from = self.sweep.wheel.dx;
                self.sweep.wheel.dx = 0.0;
                self.sweep.wheel.active = false;
                self.sweep.wheel.locked = true;
                self.sweep.spring_back(from, now);
                continue;
            }
            if !self.sweep.wheel.active && delta.x.abs() <= delta.y.abs() {
                continue;
            }
            self.sweep.wheel.active = true;
            self.sweep.settle = None;
            self.sweep.wheel.dx += delta.x;
            if self.sweep.wheel.dx.abs() >= threshold {
                let keep = self.sweep.wheel.dx > 0.0;
                self.choose(keep, now);
                self.sweep.wheel.locked = true;
            }
        }
        if self.sweep.wheel.active {
            if now - self.sweep.wheel.last > WHEEL_QUIET {
                let from = self.sweep.wheel.dx;
                self.sweep.wheel.dx = 0.0;
                self.sweep.wheel.active = false;
                self.sweep.spring_back(from, now);
            } else {
                ctx.request_repaint_after_secs(WHEEL_QUIET as f32);
            }
        }
    }

    pub fn show_sweep(&mut self, ui: &mut Ui, area: Rect, palette: Palette) {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        // A photo being flicked away must stay out of the sidebar.
        let painter = ui.painter().with_clip_rect(area);
        let has_message = self.store.library_access.is_some()
            || self.store.error.is_some()
            || self.store.status.is_some();
        let banner = if has_message { MESSAGE_ROW } else { 0.0 };
        let stage = Rect::from_min_max(
            area.min + vec2(20.0, 4.0),
            area.max - vec2(20.0, FOOTER + banner),
        );
        let threshold = (area.width() * 0.22).min(180.0);
        let interactive = !self.store.locked() && !self.store.zoom;
        let can_decide = self.can_decide() && interactive;

        let response = ui.interact(stage, ui.scope_id().with("stage"), Sense::drag());
        if can_decide {
            if response.dragged() {
                self.sweep.settle = None;
                self.sweep.drag += response.drag_delta().x;
            }
            if response.drag_stopped() {
                let speed = ctx.input(|i| i.pointer.velocity().x);
                let flick = speed.abs() > 700.0 && self.sweep.drag.abs() > 40.0;
                if self.sweep.drag.abs() >= threshold || flick {
                    let keep = self.sweep.drag > 0.0;
                    self.choose(keep, now);
                } else {
                    let from = self.sweep.drag;
                    self.sweep.drag = 0.0;
                    self.sweep.spring_back(from, now);
                }
            }
        }
        if interactive {
            self.read_trackpad(&ctx, threshold, now, can_decide);
        }

        let current = self.store.current();
        if current != self.sweep.shown {
            self.sweep.shown = current;
            self.sweep.shown_since = now;
        }
        let mut animating = false;

        if let Some(photo) = current {
            let mut dx = self.sweep.offset();
            if let Some((from, start)) = self.sweep.settle {
                let t = (now - start) as f32;
                if t < SPRING_SECONDS as f32 && !self.settings.reduce_motion {
                    dx = from * (-7.0 * t).exp() * (9.0 * t).cos();
                    animating = true;
                } else {
                    self.sweep.settle = None;
                }
            }
            let (kind, start) = self.sweep.arrival;
            let t = if self.settings.reduce_motion {
                1.0
            } else {
                ((now - start) / ARRIVE_SECONDS).clamp(0.0, 1.0) as f32
            };
            let eased = ease_out(t);
            animating |= t < 1.0;
            let (extra, scale, alpha) = match kind {
                Arrival::Soft => (0.0, 0.985 + 0.015 * eased, 0.6 + 0.4 * eased),
                Arrival::From(x) => (x * (1.0 - eased), 1.0, eased),
            };
            if let Some(texture) = self.images.preview(photo) {
                draw_photo(
                    &painter,
                    texture,
                    stage,
                    dx + extra,
                    dx * 0.012,
                    scale,
                    alpha,
                );
            } else if self.images.preview_failed(photo) {
                self.failed_card(ui, stage, photo, palette);
            } else if now - self.sweep.shown_since > SHOW_SPINNER_AFTER {
                egui::Spinner::new().paint_at(
                    ui,
                    Rect::from_center_size(stage.center() - vec2(0.0, 14.0), Vec2::splat(22.0)),
                );
                widgets::text(
                    &painter,
                    stage.center() + vec2(0.0, 16.0),
                    Align2::CENTER_CENTER,
                    "Henter bildet…",
                    12.0,
                    palette.secondary,
                );
            } else {
                ctx.request_repaint_after_secs(SHOW_SPINNER_AFTER as f32);
            }
            if dx != 0.0 {
                let progress = (dx.abs() / threshold).min(1.0);
                let (label, x) = if dx < 0.0 {
                    ("Kast", stage.left() + 74.0)
                } else {
                    ("Behold", stage.right() - 74.0)
                };
                glass_label(&painter, pos2(x, stage.center().y), label, progress);
            }
        } else {
            self.finished(ui, area, palette);
        }

        self.sweep
            .leaving
            .retain(|card| now - card.start < LEAVE_SECONDS);
        for card in &self.sweep.leaving {
            let eased = ease_out(((now - card.start) / LEAVE_SECONDS) as f32);
            animating = true;
            let Some(texture) = &card.texture else {
                continue;
            };
            match card.direction {
                Direction::Down => {
                    let drawn = stage.translate(vec2(0.0, stage.height() * 0.5 * eased));
                    draw_photo(
                        &painter,
                        texture,
                        drawn,
                        0.0,
                        0.0,
                        1.0 - 0.08 * eased,
                        1.0 - eased,
                    );
                }
                direction => {
                    let side = if direction == Direction::Right {
                        1.0
                    } else {
                        -1.0
                    };
                    let distance = stage.width() * 0.75;
                    let x = card.from + (side * distance - card.from) * eased;
                    let angle = card.from * 0.012 + (side * 7.0 - card.from * 0.012) * eased;
                    draw_photo(&painter, texture, stage, x, angle, 1.0, 1.0 - eased);
                }
            }
        }

        if has_message {
            let row = Rect::from_min_max(
                pos2(area.left() + 20.0, stage.bottom() + 5.0),
                pos2(area.right() - 20.0, stage.bottom() + MESSAGE_ROW - 3.0),
            );
            self.status_line(ui, row, palette);
        }
        if current.is_some() {
            self.footer(ui, area, palette, can_decide, now);
        }
        if animating {
            ctx.request_repaint();
        }
    }

    fn failed_card(&mut self, ui: &mut Ui, stage: Rect, photo: usize, palette: Palette) {
        let painter = ui.painter().clone();
        let center = stage.center();
        widgets::icon(
            &painter,
            center - vec2(0.0, 46.0),
            Icon::Photo,
            palette.tertiary,
        );
        widgets::text(
            &painter,
            center - vec2(0.0, 22.0),
            Align2::CENTER_CENTER,
            "Bildet kunne ikke vises",
            13.0,
            palette.label,
        );
        let title = photo_title(&self.store.photos()[photo]);
        widgets::text(
            &painter,
            center,
            Align2::CENTER_CENTER,
            title,
            12.0,
            palette.secondary,
        );
        if self.store.is_library() {
            let rect = Rect::from_center_size(center + vec2(0.0, 36.0), vec2(100.0, 28.0));
            if push_button(
                ui,
                rect,
                "retry",
                "Prøv igjen",
                Style::Plain,
                true,
                false,
                palette,
            ) {
                self.images.retry_preview(photo);
                self.sweep.shown = None;
            }
        }
    }

    fn finished(&mut self, ui: &mut Ui, area: Rect, palette: Palette) {
        let (_, keep, discard) = self.store.counts();
        let center = area.center();
        let painter = ui.painter().clone();
        let total = keep + discard;
        if total == 0 {
            widgets::text(
                &painter,
                center - vec2(0.0, 30.0),
                Align2::CENTER_CENTER,
                "Ingen bilder her",
                17.0,
                palette.label,
            );
            widgets::text(
                &painter,
                center - vec2(0.0, 4.0),
                Align2::CENTER_CENTER,
                "Mappen inneholder ingen bilder som kan sorteres.",
                13.0,
                palette.secondary,
            );
            let rect = Rect::from_center_size(center + vec2(0.0, 36.0), vec2(118.0, 28.0));
            if push_button(
                ui,
                rect,
                "choose",
                "Velg mappe…",
                Style::Plain,
                !self.store.locked(),
                false,
                palette,
            ) {
                self.pick_folder();
            }
            return;
        }
        widgets::text(
            &painter,
            center - vec2(0.0, 30.0),
            Align2::CENTER_CENTER,
            "Alle bildene er sortert",
            17.0,
            palette.label,
        );
        let detail = if discard > 0 {
            format!("{} venter på å bli gjennomgått.", images(discard))
        } else {
            "Ingen bilder er merket for sletting.".to_owned()
        };
        widgets::text(
            &painter,
            center - vec2(0.0, 4.0),
            Align2::CENTER_CENTER,
            detail,
            13.0,
            palette.secondary,
        );
        if discard > 0 {
            let label = "Gå gjennom til sletting";
            let rect = Rect::from_center_size(
                center + vec2(0.0, 36.0),
                vec2(widgets::button_width(ui, label), 28.0),
            );
            if push_button(
                ui,
                rect,
                "review",
                label,
                Style::Prominent,
                !self.store.locked(),
                false,
                palette,
            ) {
                self.store.set_view(crate::store::View::Review);
            }
        } else {
            let rect = Rect::from_center_size(center + vec2(0.0, 36.0), vec2(118.0, 28.0));
            if push_button(
                ui,
                rect,
                "choose",
                "Velg mappe…",
                Style::Plain,
                !self.store.locked(),
                false,
                palette,
            ) {
                self.pick_folder();
            }
        }
    }

    fn footer(&mut self, ui: &mut Ui, area: Rect, palette: Palette, enabled: bool, now: f64) {
        let y = area.bottom() - FOOTER / 2.0;
        let bar = Rect::from_center_size(pos2(area.center().x, y), vec2(204.0, 40.0));
        let painter = ui.painter().clone();
        let left_end = bar.left() - 16.0;
        let details = widgets::fit_text(
            ui,
            &self.current_details(),
            12.0,
            left_end - area.left() - 20.0,
        );
        widgets::text(
            &painter,
            pos2(area.left() + 20.0, y),
            Align2::LEFT_CENTER,
            details,
            12.0,
            palette.secondary,
        );
        glass_pill(&painter, bar);
        painter.vline(
            bar.center().x,
            (bar.top() + 10.0)..=(bar.bottom() - 10.0),
            Stroke::new(1.0, Color32::from_white_alpha(46)),
        );
        let left = Rect::from_min_max(bar.min, pos2(bar.center().x, bar.max.y));
        let right = Rect::from_min_max(pos2(bar.center().x, bar.min.y), bar.max);
        if glass_button(ui, left, "discard", Icon::Trash, "Kast", enabled) {
            self.choose(false, now);
        }
        if glass_button(ui, right, "keep", Icon::Check, "Behold", enabled) {
            self.choose(true, now);
        }
    }

    /// The quiet details of the current photo.
    pub fn current_details(&self) -> String {
        let Some(photo) = self.store.current() else {
            return String::new();
        };
        let photo = &self.store.photos()[photo];
        if self.store.is_library() {
            photo_title(photo)
        } else {
            format!(
                "{}    {}    {}",
                photo.name,
                bytes(photo.size),
                when(photo.modified)
            )
        }
    }
}

fn ease_out(t: f32) -> f32 {
    let inverse = 1.0 - t.clamp(0.0, 1.0);
    1.0 - inverse * inverse * inverse
}

/// Draws a photo fitted inside `area`, moved `dx` sideways and tilted `degrees`.
pub fn draw_photo(
    painter: &Painter,
    texture: &TextureHandle,
    area: Rect,
    dx: f32,
    degrees: f32,
    scale: f32,
    alpha: f32,
) {
    let size = texture.size_vec2();
    let fit = (area.width() / size.x).min(area.height() / size.y) * scale;
    let center = area.center() + vec2(dx, 0.0);
    let rect = Rect::from_center_size(center, size * fit);
    let mut mesh = Mesh::with_texture(texture.id());
    let uv = Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0));
    mesh.add_rect_with_uv(
        rect,
        uv,
        Color32::WHITE.gamma_multiply(alpha.clamp(0.0, 1.0)),
    );
    if degrees != 0.0 {
        mesh.rotate(Rot2::from_angle(degrees.to_radians()), center);
    }
    painter.add(Shape::mesh(mesh));
}
