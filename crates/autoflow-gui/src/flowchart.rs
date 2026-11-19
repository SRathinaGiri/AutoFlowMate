//! Graphical workflow view: lays the step tree out as a flowchart.
//!
//! * plain steps are boxes joined by arrows,
//! * `if` is a decision node with yes/no branches that merge again,
//! * `repeat` / `while` / `for_each` are frames with a loop-back arrow,
//! * `try` is a frame with its steps, then the `on error` and `finally` lists,
//! * every connector is a drop zone: drag an action from the palette (or an
//!   existing node) onto it, or click its ⊕ to make it the insertion point.

use autoflow_core::{Action, Step};
use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Vec2};
use std::collections::HashSet;

use crate::tree::{DragItem, ListPath, Selection, StepPath};

const NODE_W: f32 = 240.0;
const NODE_H: f32 = 52.0;
const GAP: f32 = 36.0;
const BR_GAP: f32 = 36.0;
const IF_TOP: f32 = 30.0;
const IF_BOT: f32 = 26.0;
const LOOP_PAD: f32 = 16.0;
const LOOP_LEFT: f32 = 30.0;
const EMPTY_W: f32 = 150.0;
const MARGIN: f32 = 40.0;
const TERM_H: f32 = 28.0;
const LABEL_H: f32 = 22.0;

pub enum ChartOp {
    Select(Selection),
    Drop(DragItem, ListPath, usize),
    Toggle(StepPath),
    Delete(StepPath),
    Duplicate(StepPath),
    Breakpoint(StepPath),
}

pub struct Chart<'a> {
    pub sel: &'a Selection,
    pub breakpoints: &'a HashSet<String>,
    /// The step a debug run is paused before.
    pub paused: Option<&'a str>,
    pub current: Option<&'a str>,
    pub failed: &'a [String],
    pub zoom: f32,
    pub color: fn(&str) -> Color32,
    pub summary: fn(&Step) -> String,
}

impl Chart<'_> {
    pub fn show(&self, ui: &mut egui::Ui, steps: &[Step], ops: &mut Vec<ChartOp>) {
        let z = self.zoom;
        let body = measure_list(steps);
        let size = Vec2::new(body.x.max(NODE_W) + 2.0 * MARGIN, body.y + 2.0 * (TERM_H + MARGIN)) * z;
        let avail = ui.available_size();
        let (rect, _) = ui.allocate_exact_size(Vec2::new(size.x.max(avail.x), size.y.max(avail.y)), Sense::hover());
        let painter = ui.painter_at(rect);
        let cx = rect.center().x;
        let mut y = rect.top() + MARGIN * z;

        let p = Painter { ui, painter: &painter, chart: self, z };
        p.terminal(Pos2::new(cx, y + TERM_H * z / 2.0), "Start");
        y += TERM_H * z;
        let end = p.list(steps, &vec![], cx, y, ops);
        p.terminal(Pos2::new(cx, end + TERM_H * z / 2.0), "End");
    }
}

// ------------------------------------------------------------------ layout

fn is_loop(a: &Action) -> bool {
    matches!(a, Action::Repeat(_) | Action::While(_) | Action::ForEach(_))
}

fn measure_list(steps: &[Step]) -> Vec2 {
    if steps.is_empty() {
        return Vec2::new(EMPTY_W, GAP);
    }
    let mut w: f32 = 0.0;
    let mut h = GAP;
    for s in steps {
        let m = measure_step(s);
        w = w.max(m.x);
        h += m.y + GAP;
    }
    Vec2::new(w, h)
}

fn measure_step(s: &Step) -> Vec2 {
    match &s.action {
        Action::If(a) => {
            let (t, e) = (measure_list(&a.then), measure_list(&a.otherwise));
            Vec2::new(NODE_W.max(t.x + e.x + BR_GAP), NODE_H + IF_TOP + t.y.max(e.y) + IF_BOT)
        }
        Action::Try(a) => {
            let (t, c, f) = (measure_list(&a.steps), measure_list(&a.catch), measure_list(&a.finally));
            Vec2::new(NODE_W.max(t.x).max(c.x).max(f.x) + 2.0 * LOOP_PAD, LOOP_PAD + NODE_H + t.y + c.y + f.y + 2.0 * LABEL_H + LOOP_PAD)
        }
        a if is_loop(a) => {
            let b = measure_list(a.children()[0]);
            Vec2::new(NODE_W.max(b.x) + LOOP_LEFT + 2.0 * LOOP_PAD, LOOP_PAD + NODE_H + b.y + LOOP_PAD)
        }
        _ => Vec2::new(NODE_W, NODE_H),
    }
}

// ------------------------------------------------------------------ painting

struct Painter<'a, 'u> {
    ui: &'a egui::Ui,
    painter: &'a egui::Painter,
    chart: &'a Chart<'u>,
    z: f32,
}

impl Painter<'_, '_> {
    fn line_color(&self) -> Color32 {
        self.ui.visuals().widgets.noninteractive.fg_stroke.color.gamma_multiply(0.7)
    }

    fn stroke(&self) -> Stroke {
        Stroke::new(1.5 * self.z, self.line_color())
    }

    fn vline(&self, x: f32, y0: f32, y1: f32) {
        self.painter.line_segment([Pos2::new(x, y0), Pos2::new(x, y1)], self.stroke());
    }

    fn hline(&self, y: f32, x0: f32, x1: f32) {
        self.painter.line_segment([Pos2::new(x0, y), Pos2::new(x1, y)], self.stroke());
    }

    fn arrow_head(&self, tip: Pos2, dir: Vec2) {
        let d = dir.normalized() * 7.0 * self.z;
        let n = Vec2::new(-d.y, d.x) * 0.6;
        self.painter.add(Shape::convex_polygon(vec![tip, tip - d + n, tip - d - n], self.line_color(), Stroke::NONE));
    }

    fn terminal(&self, center: Pos2, label: &str) {
        let r = Rect::from_center_size(center, Vec2::new(90.0, TERM_H) * self.z);
        let v = self.ui.visuals();
        self.painter.rect_filled(r, CornerRadius::same((TERM_H * self.z / 2.0) as u8), v.widgets.inactive.bg_fill);
        self.painter.rect_stroke(r, CornerRadius::same((TERM_H * self.z / 2.0) as u8), self.stroke(), StrokeKind::Inside);
        self.painter.text(center, Align2::CENTER_CENTER, label, FontId::proportional(13.0 * self.z), v.text_color());
    }

    /// Draw a step list centred at `cx`, starting at `top`. Returns the bottom y.
    fn list(&self, steps: &[Step], lp: &ListPath, cx: f32, top: f32, ops: &mut Vec<ChartOp>) -> f32 {
        let z = self.z;
        let width = measure_list(steps).x * z;
        let mut y = top;
        self.gap(cx, y, width, lp, 0, steps.is_empty(), ops);
        y += GAP * z;
        for (i, s) in steps.iter().enumerate() {
            let path = StepPath { parents: lp.clone(), index: i };
            let h = measure_step(s).y * z;
            self.step(s, &path, cx, y, ops);
            y += h;
            self.gap(cx, y, width, lp, i + 1, false, ops);
            y += GAP * z;
        }
        y
    }

    /// A connector: line + arrow, a ⊕ button and a drop zone.
    #[allow(clippy::too_many_arguments)]
    fn gap(&self, cx: f32, y: f32, width: f32, lp: &ListPath, idx: usize, empty: bool, ops: &mut Vec<ChartOp>) {
        let z = self.z;
        let h = GAP * z;
        self.vline(cx, y, y + h - 1.0);
        self.arrow_head(Pos2::new(cx, y + h), Vec2::new(0.0, 1.0));
        let zone = Rect::from_center_size(Pos2::new(cx, y + h / 2.0), Vec2::new(width.max(NODE_W * z), h));
        let id = self.ui.scope_id().with(("gap", lp.clone(), idx));
        let resp = self.ui.interact(zone, id, Sense::click());
        let selected = *self.chart.sel == Selection::Gap(lp.clone(), idx);
        let accent = self.ui.visuals().selection.stroke.color;

        let hovering_drag = resp.dnd_hover_payload::<DragItem>().is_some();
        if let Some(item) = resp.dnd_release_payload::<DragItem>() {
            ops.push(ChartOp::Drop((*item).clone(), lp.clone(), idx));
        }
        if empty {
            let r = Rect::from_center_size(zone.center(), Vec2::new(EMPTY_W * z * 0.9, h * 0.7));
            let col = if hovering_drag || selected { accent } else { self.line_color().gamma_multiply(0.6) };
            self.painter.rect_stroke(r, CornerRadius::same(6), Stroke::new(1.0, col), StrokeKind::Inside);
            self.painter.text(r.center(), Align2::CENTER_CENTER, "drop steps here", FontId::proportional(11.0 * z), col);
        }
        if hovering_drag {
            let w = width.max(NODE_W * z) * 0.8;
            self.painter.line_segment([Pos2::new(cx - w / 2.0, zone.center().y), Pos2::new(cx + w / 2.0, zone.center().y)], Stroke::new(3.0 * z, accent));
        } else if !empty && (resp.hovered() || selected) {
            let c = zone.center();
            let r = 9.0 * z;
            self.painter.circle_filled(c, r, if selected { accent } else { self.ui.visuals().widgets.hovered.bg_fill });
            self.painter.circle_stroke(c, r, Stroke::new(1.0, accent));
            self.painter.text(c, Align2::CENTER_CENTER, "+", FontId::proportional(14.0 * z), self.ui.visuals().strong_text_color());
        }
        if resp.clicked() {
            ops.push(ChartOp::Select(Selection::Gap(lp.clone(), idx)));
        }
        resp.on_hover_text("Click to insert here from the palette, or drop an action / step here");
    }

    fn step(&self, s: &Step, path: &StepPath, cx: f32, top: f32, ops: &mut Vec<ChartOp>) {
        let z = self.z;
        match &s.action {
            Action::If(a) => {
                let node = Rect::from_min_size(Pos2::new(cx - NODE_W * z / 2.0, top), Vec2::new(NODE_W, NODE_H) * z);
                self.node(s, path, node, NodeShape::Decision, ops);
                let (t, e) = (measure_list(&a.then) * z, measure_list(&a.otherwise) * z);
                let total = t.x + e.x + BR_GAP * z;
                let lx = cx - total / 2.0;
                let (ct, ce) = (lx + t.x / 2.0, lx + t.x + BR_GAP * z + e.x / 2.0);
                let ym = node.bottom() + IF_TOP * z / 2.0;
                let yb = node.bottom() + IF_TOP * z;
                self.vline(cx, node.bottom(), ym);
                self.hline(ym, ct, ce);
                self.vline(ct, ym, yb);
                self.vline(ce, ym, yb);
                let f = FontId::proportional(11.0 * z);
                self.painter.text(Pos2::new(ct + 4.0, ym + 2.0), Align2::LEFT_TOP, "yes", f.clone(), Color32::from_rgb(90, 180, 100));
                self.painter.text(Pos2::new(ce + 4.0, ym + 2.0), Align2::LEFT_TOP, "no", f, Color32::from_rgb(220, 100, 100));
                let mut lp_then = path.parents.clone();
                lp_then.push((path.index, 0));
                let mut lp_else = path.parents.clone();
                lp_else.push((path.index, 1));
                let bt = self.list(&a.then, &lp_then, ct, yb, ops);
                let be = self.list(&a.otherwise, &lp_else, ce, yb, ops);
                let total_h = measure_step(s).y * z;
                let merge = top + total_h - IF_BOT * z / 2.0;
                self.vline(ct, bt, merge);
                self.vline(ce, be, merge);
                self.hline(merge, ct, ce);
                self.vline(cx, merge, top + total_h);
                self.painter.circle_filled(Pos2::new(cx, merge), 3.5 * z, self.line_color());
            }
            Action::Try(a) => {
                let size = measure_step(s) * z;
                let frame = Rect::from_min_size(Pos2::new(cx - size.x / 2.0, top), size);
                let col = (self.chart.color)(s.action.category());
                self.painter.rect_filled(frame, CornerRadius::same(10), col.gamma_multiply(0.07));
                self.painter.rect_stroke(frame, CornerRadius::same(10), Stroke::new(1.0, col.gamma_multiply(0.5)), StrokeKind::Inside);
                let node = Rect::from_min_size(Pos2::new(cx - NODE_W * z / 2.0, top + LOOP_PAD * z), Vec2::new(NODE_W, NODE_H) * z);
                self.vline(cx, top, node.top());
                self.node(s, path, node, NodeShape::Loop, ops);
                let mut y = node.bottom();
                let sections: [(&[Step], &str, Color32); 3] = [
                    (&a.steps, "", Color32::TRANSPARENT),
                    (&a.catch, "on error", Color32::from_rgb(220, 100, 100)),
                    (&a.finally, "finally (always)", Color32::from_rgb(120, 150, 220)),
                ];
                for (i, (steps, label, color)) in sections.into_iter().enumerate() {
                    if !label.is_empty() {
                        let ym = y + LABEL_H * z / 2.0;
                        self.painter.line_segment(
                            [Pos2::new(frame.left() + 8.0 * z, ym), Pos2::new(frame.right() - 8.0 * z, ym)],
                            Stroke::new(1.0, color.gamma_multiply(0.6)),
                        );
                        let galley_pos = Pos2::new(cx + 6.0 * z, ym - 1.0);
                        self.painter.text(galley_pos, Align2::LEFT_BOTTOM, label, FontId::proportional(11.0 * z), color);
                        self.vline(cx, y, y + LABEL_H * z);
                        y += LABEL_H * z;
                    }
                    let mut lp = path.parents.clone();
                    lp.push((path.index, i));
                    y = self.list(steps, &lp, cx, y, ops);
                }
                self.vline(cx, y, frame.bottom());
            }
            a if is_loop(a) => {
                let size = measure_step(s) * z;
                let frame = Rect::from_min_size(Pos2::new(cx - size.x / 2.0, top), size);
                let col = (self.chart.color)(a.category());
                self.painter.rect_filled(frame, CornerRadius::same(10), col.gamma_multiply(0.07));
                self.painter.rect_stroke(frame, CornerRadius::same(10), Stroke::new(1.0, col.gamma_multiply(0.5)), StrokeKind::Inside);
                let bx = cx + LOOP_LEFT * z / 2.0;
                let node = Rect::from_min_size(Pos2::new(bx - NODE_W * z / 2.0, top + LOOP_PAD * z), Vec2::new(NODE_W, NODE_H) * z);
                // entry connector from the frame edge to the header
                self.painter.line_segment([Pos2::new(cx, top), Pos2::new(bx, node.top())], self.stroke());
                self.node(s, path, node, NodeShape::Loop, ops);
                let mut lp = path.parents.clone();
                lp.push((path.index, 0));
                let end = self.list(a.children()[0], &lp, bx, node.bottom(), ops);
                // loop-back arrow
                let lx = frame.left() + LOOP_LEFT * z / 2.0;
                let yb = frame.bottom() - LOOP_PAD * z / 2.0;
                self.vline(bx, end, yb);
                self.hline(yb, lx, bx);
                self.vline(lx, node.center().y, yb);
                self.hline(node.center().y, lx, node.left() - 1.0);
                self.arrow_head(Pos2::new(node.left(), node.center().y), Vec2::new(1.0, 0.0));
                self.painter.text(Pos2::new(lx + 3.0, yb - 3.0), Align2::LEFT_BOTTOM, "loop", FontId::proportional(10.0 * z), self.line_color());
                // exit
                self.painter.line_segment([Pos2::new(bx, yb), Pos2::new(cx, frame.bottom())], self.stroke());
            }
            _ => {
                let node = Rect::from_min_size(Pos2::new(cx - NODE_W * z / 2.0, top), Vec2::new(NODE_W, NODE_H) * z);
                self.node(s, path, node, NodeShape::Box, ops);
            }
        }
    }

    fn node(&self, s: &Step, path: &StepPath, r: Rect, shape: NodeShape, ops: &mut Vec<ChartOp>) {
        let z = self.z;
        let v = self.ui.visuals().clone();
        let cat_col = (self.chart.color)(s.action.category());
        let selected = *self.chart.sel == Selection::Step(path.clone());
        let running = !s.id.is_empty() && self.chart.current == Some(s.id.as_str());
        let failed = self.chart.failed.iter().any(|f| f == &s.id);
        let breakpoint = !s.id.is_empty() && self.chart.breakpoints.contains(&s.id);
        let paused = !s.id.is_empty() && self.chart.paused == Some(s.id.as_str());
        let dim = if s.enabled { 1.0 } else { 0.4 };

        let id = self.ui.scope_id().with(("node", path.parents.clone(), path.index));
        let resp = self.ui.interact(r, id, Sense::click_and_drag());
        resp.dnd_set_drag_payload(DragItem::Move(path.clone()));

        let fill = if resp.hovered() { v.widgets.hovered.bg_fill } else { v.widgets.inactive.bg_fill };
        let border = if paused {
            Stroke::new(3.0 * z, Color32::from_rgb(235, 170, 40))
        } else if running {
            Stroke::new(3.0 * z, Color32::from_rgb(80, 150, 240))
        } else if failed {
            Stroke::new(2.5 * z, Color32::from_rgb(225, 80, 80))
        } else if selected {
            Stroke::new(2.5 * z, v.selection.stroke.color)
        } else {
            Stroke::new(1.0 * z, cat_col.gamma_multiply(0.7 * dim))
        };
        match shape {
            NodeShape::Decision => {
                let k = 16.0 * z;
                let pts = vec![
                    Pos2::new(r.left() + k, r.top()),
                    Pos2::new(r.right() - k, r.top()),
                    Pos2::new(r.right(), r.center().y),
                    Pos2::new(r.right() - k, r.bottom()),
                    Pos2::new(r.left() + k, r.bottom()),
                    Pos2::new(r.left(), r.center().y),
                ];
                self.painter.add(Shape::convex_polygon(pts, fill.gamma_multiply(dim), border));
            }
            _ => {
                let cr = CornerRadius::same(if matches!(shape, NodeShape::Loop) { 14 } else { 7 });
                self.painter.rect_filled(r, cr, fill.gamma_multiply(dim));
                let stripe = Rect::from_min_max(r.min, Pos2::new(r.left() + 5.0 * z, r.bottom()));
                self.painter.rect_filled(stripe, CornerRadius { nw: cr.nw, sw: cr.sw, ne: 0, se: 0 }, cat_col.gamma_multiply(dim));
                self.painter.rect_stroke(r, cr, border, StrokeKind::Inside);
            }
        }

        let pad = if matches!(shape, NodeShape::Decision) { 22.0 } else { 13.0 } * z;
        let title = match &s.action {
            Action::If(_) => format!("{} if", crate::icons::action_icon("if")),
            a => format!("{} {}", crate::icons::action_icon(a.name()), a.name()),
        };
        let mut title_text = title;
        if paused {
            title_text = format!("⏸ {title_text}");
        } else if running {
            title_text = format!("▶ {title_text}");
        } else if failed {
            title_text = format!("✖ {title_text}");
        }
        self.painter.text(
            Pos2::new(r.left() + pad, r.top() + 7.0 * z),
            Align2::LEFT_TOP,
            title_text,
            FontId::proportional(13.5 * z),
            cat_col.gamma_multiply(dim),
        );
        if !s.id.is_empty() {
            self.painter.text(Pos2::new(r.right() - pad * 0.8, r.top() + 7.0 * z), Align2::RIGHT_TOP, &s.id, FontId::monospace(10.0 * z), v.weak_text_color());
        }
        let summary = (self.chart.summary)(s);
        let max_chars = ((r.width() - 2.0 * pad) / (6.3 * z)).max(4.0) as usize;
        let short: String =
            if summary.chars().count() > max_chars { summary.chars().take(max_chars.saturating_sub(1)).collect::<String>() + "…" } else { summary.clone() };
        self.painter.text(
            Pos2::new(r.left() + pad, r.bottom() - 7.0 * z),
            Align2::LEFT_BOTTOM,
            short,
            FontId::proportional(11.5 * z),
            v.text_color().gamma_multiply(0.75 * dim),
        );
        if !s.enabled {
            self.painter.text(
                r.right_bottom() - Vec2::new(pad * 0.8, 7.0 * z),
                Align2::RIGHT_BOTTOM,
                "off",
                FontId::proportional(10.0 * z),
                v.weak_text_color(),
            );
        }
        if breakpoint {
            let c = Pos2::new(r.left() - 9.0 * z, r.center().y);
            self.painter.circle_filled(c, 6.0 * z, Color32::from_rgb(220, 60, 60));
            self.painter.circle_stroke(c, 6.0 * z, Stroke::new(1.0, Color32::from_rgb(140, 30, 30)));
        }

        if resp.clicked() || resp.drag_started() {
            ops.push(ChartOp::Select(Selection::Step(path.clone())));
        }
        let mut tip = summary;
        if !s.comment.is_empty() {
            tip = format!("{tip}\n// {}", s.comment);
        }
        let resp = resp.on_hover_text(format!("{tip}\n\nDrag to move • right-click for options"));
        resp.context_menu(|ui| {
            if ui.button(if s.enabled { "Disable" } else { "Enable" }).clicked() {
                ops.push(ChartOp::Toggle(path.clone()));
                ui.close();
            }
            if ui.button("Duplicate").clicked() {
                ops.push(ChartOp::Duplicate(path.clone()));
                ui.close();
            }
            if ui.button(if breakpoint { "Remove breakpoint (F9)" } else { "Add breakpoint (F9)" }).clicked() {
                ops.push(ChartOp::Breakpoint(path.clone()));
                ui.close();
            }
            if ui.button("Delete").clicked() {
                ops.push(ChartOp::Delete(path.clone()));
                ui.close();
            }
        });
    }
}

#[derive(Clone, Copy)]
enum NodeShape {
    Box,
    Decision,
    Loop,
}
