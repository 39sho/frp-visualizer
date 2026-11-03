use crate::execution::LaneSpec;
use crate::model::{
    fit_text, text_width, AnnotationTarget, Document, NodeId, NodeKind, PortType, Rect,
};
use crate::scene::{
    arrow_head, Color, Scene, Stroke, TextAnchor, ACCENT, BLUE, CELL_INK, FAINT, INK, MUTED, WHITE,
    YELLOW,
};
use crate::sim::{SimStep, Simulation, TransactionTrace};
use std::collections::HashSet;

const LABEL_W: f32 = 220.0;
const HEADER_H: f32 = 58.0;
const ROW_H: f32 = 66.0;
const FOOTER_H: f32 = 52.0;

#[derive(Clone, Debug)]
pub struct TimingView {
    pub scene: Scene,
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TimingNetworkMode {
    #[default]
    Focus,
    None,
}

#[derive(Clone, Debug)]
pub struct TimingOptions {
    pub width: f32,
    pub show_transactions: bool,
    pub show_values: bool,
    pub focus: Option<usize>,
    pub timing_network: TimingNetworkMode,
}

impl Default for TimingOptions {
    fn default() -> Self {
        Self {
            width: 1160.0,
            show_transactions: true,
            show_values: true,
            focus: None,
            timing_network: TimingNetworkMode::Focus,
        }
    }
}

pub fn render_timing(
    doc: &Document,
    sim: &Simulation,
    lanes: &[LaneSpec],
    options: &TimingOptions,
    show_annotations: bool,
    focus_doc: &Document,
) -> TimingView {
    let mut scene = Scene::new();
    let width = options.width.max(760.0);
    let label_w = LABEL_W;
    let x0 = label_w + 24.0;
    let x1 = width - 46.0;
    let first_row_y = HEADER_H;
    let row_h = ROW_H;
    let end = doc.effective_end_time().max(1.0);
    let map_x = |t: f64| -> f32 { x0 + (t / end).clamp(0.0, 1.2) as f32 * (x1 - x0) };

    let lane_count = lanes.len().max(1);
    let lanes_bottom = HEADER_H + lane_count as f32 * row_h;
    let main_height = lanes_bottom + FOOTER_H;
    let axis_y = main_height - 32.0;

    let exec_panel = if options.timing_network == TimingNetworkMode::Focus {
        options.focus.and_then(|index| {
            let trace = sim.traces.get(index)?;
            let focus = build_focus_network(focus_doc, trace)?;
            let scale = ((width - 80.0) / focus.bounds.w.max(1.0))
                .min(88.0 / focus.bounds.h.max(1.0))
                .clamp(0.2, 1.0);
            let panel_height = 34.0 + focus.bounds.h * scale + 28.0;
            Some((trace, focus, scale, panel_height))
        })
    } else {
        None
    };
    let exec_height = exec_panel
        .as_ref()
        .map(|panel| panel.3 + 12.0)
        .unwrap_or(0.0);
    let height = main_height + exec_height;

    let tx_xs: Vec<f32> = sim.transactions.iter().map(|t| map_x(*t)).collect();
    let tx_ends: Vec<f32> = tx_xs
        .iter()
        .enumerate()
        .map(|(index, x)| {
            let next = tx_xs.get(index + 1).copied().unwrap_or(x1);
            let gap = ((next - x) * 0.4).clamp(2.0, 8.0);
            (x + gap).min(x1 - 1.0)
        })
        .collect();
    let transaction_end = |time: f64| -> f32 {
        sim.transactions
            .iter()
            .position(|t| (t - time).abs() < 1e-9)
            .and_then(|index| tx_ends.get(index).copied())
            .unwrap_or_else(|| map_x(time))
    };

    scene.text([20.0, 18.0], "transactions", 10.5, MUTED, TextAnchor::Start);
    scene.text(
        [20.0, 33.0],
        "Tn start · cells update at Tn end",
        9.5,
        FAINT,
        TextAnchor::Start,
    );
    if options.show_transactions {
        for (i, t) in sim.transactions.iter().enumerate() {
            let x = map_x(*t);
            scene.line(
                [x, 50.0],
                [x, axis_y - 6.0],
                Stroke::dashed(FAINT, 1.0, 3.0, 4.0),
            );
            if let Some(&end_x) = tx_ends.get(i) {
                if end_x > x + 1.0 {
                    scene.line([x, 50.0], [end_x, 50.0], Stroke::new(FAINT, 0.9));
                    scene.line([end_x, 50.0], [end_x, 54.0], Stroke::new(FAINT, 0.9));
                }
            }
            if sim.transactions.len() <= 16 || i % 2 == 0 {
                scene.text(
                    [x, 42.0],
                    format!("T{}", i + 1),
                    10.5,
                    MUTED,
                    TextAnchor::Center,
                );
            }
        }
    }
    scene.line(
        [label_w + 10.0, HEADER_H - 6.0],
        [label_w + 10.0, axis_y - 6.0],
        Stroke::new(crate::scene::Color(0, 0, 0, 20), 1.0),
    );

    for (index, lane) in lanes.iter().enumerate() {
        let row_y = first_row_y + index as f32 * row_h;
        if !index.is_multiple_of(2) {
            scene.rect(
                Rect::new(0.0, row_y, width, row_h),
                0.0,
                crate::scene::Color(0, 0, 0, 7),
                None,
            );
        }
        scene.line(
            [16.0, row_y + row_h],
            [width - 16.0, row_y + row_h],
            Stroke::new(crate::scene::Color(0, 0, 0, 18), 1.0),
        );
        let baseline = row_y + 33.0;

        let is_cell = lane.kind == PortType::Cell;
        let kind = if is_cell { "cell" } else { "stream" };
        let kind_color = if is_cell { CELL_INK } else { MUTED };
        let swatch_y = row_y + 20.0;
        scene.rect(
            Rect::new(20.0, swatch_y - 5.0, 10.0, 10.0),
            2.0,
            if is_cell { BLUE } else { YELLOW },
            Some(Stroke::new(INK, 1.0)),
        );
        let name_max = label_w - 38.0 - text_width(kind, 9.5) - 12.0;
        scene.bold_text(
            [38.0, row_y + 21.0],
            truncate(&lane.label, name_max, 13.5),
            13.5,
            INK,
            TextAnchor::Start,
        );
        scene.text(
            [label_w - 8.0, row_y + 21.0],
            kind,
            9.5,
            kind_color,
            TextAnchor::End,
        );

        let timeline_color = if is_cell { CELL_INK } else { INK };
        scene.line(
            [x0, baseline],
            [x1, baseline],
            Stroke::new(timeline_color, 1.4),
        );
        let head = arrow_head([x1, baseline], [1.0, 0.0], 8.0);
        scene.path(head, true, timeline_color, None);

        if is_cell {
            draw_cell(
                &mut scene,
                sim.cell(lane.source),
                x0,
                x1,
                baseline,
                transaction_end,
                options.show_values,
            );
        } else {
            draw_stream(
                &mut scene,
                sim.stream(lane.source),
                x0,
                x1,
                baseline,
                map_x,
                options.show_values,
            );
        }
    }

    scene.line([x0, axis_y], [x1, axis_y], Stroke::new(INK, 1.3));
    let head = arrow_head([x1, axis_y], [1.0, 0.0], 8.0);
    scene.path(head, true, INK, None);
    let mut tick = 1.0;
    if end > 12.0 {
        tick = 2.0;
    }
    if end > 30.0 {
        tick = 5.0;
    }
    let mut t = 0.0;
    while t <= end + 1e-6 {
        let x = map_x(t);
        scene.line([x, axis_y - 4.0], [x, axis_y + 4.0], Stroke::new(INK, 1.0));
        scene.text(
            [x, axis_y + 15.0],
            format!("{}", t as i64),
            10.5,
            MUTED,
            TextAnchor::Center,
        );
        t += tick;
    }
    scene.text(
        [x1 - 4.0, axis_y - 13.0],
        "time →",
        11.0,
        MUTED,
        TextAnchor::End,
    );

    if let Some((trace, focus, scale, panel_height)) = &exec_panel {
        let panel_top = main_height + 6.0;
        scene.rect(
            Rect::new(20.0, panel_top, width - 40.0, *panel_height),
            8.0,
            WHITE,
            Some(Stroke::new(FAINT, 1.0)),
        );
        scene.text(
            [32.0, panel_top + 15.0],
            format!("Execution in time · T{}", trace.index + 1),
            10.5,
            MUTED,
            TextAnchor::Start,
        );
        let dx = (width - focus.bounds.w * scale) / 2.0 - focus.bounds.x * scale;
        let dy = panel_top + 34.0 - focus.bounds.y * scale;
        scene.append_scaled(&focus.scene, dx, dy, *scale);

        for (&id, rect) in &focus.layout.node_rects {
            if !focus.active.contains(&id) {
                continue;
            }
            let scaled = Rect::new(
                rect.x * scale + dx,
                rect.y * scale + dy,
                rect.w * scale,
                rect.h * scale,
            );
            let is_snapshot = focus.snapshots.contains(&id);
            let is_hold = focus.holds.contains(&id);
            let stroke = if is_snapshot {
                Stroke::dashed(ACCENT, 1.2, 3.0, 3.0)
            } else {
                Stroke::new(ACCENT, 1.3)
            };
            scene.rect(scaled.expand(3.0), 6.0, Color(0, 0, 0, 0), Some(stroke));
            if is_snapshot {
                scene.text(
                    [scaled.right() + 6.0, scaled.center()[1]],
                    "snap!",
                    8.5,
                    ACCENT,
                    TextAnchor::Start,
                );
            }
            if is_hold {
                scene.text(
                    [scaled.center()[0], scaled.bottom() + 9.0],
                    "update",
                    8.5,
                    ACCENT,
                    TextAnchor::Center,
                );
            }
            let order = trace
                .event_order
                .iter()
                .position(|node| *node == id)
                .or_else(|| trace.update_order.iter().position(|node| *node == id));
            if let Some(index) = order {
                scene.text(
                    [scaled.x + 2.0, scaled.y - 2.0],
                    format!("#{}", index + 1),
                    8.0,
                    ACCENT,
                    TextAnchor::Start,
                );
            }
        }

        if let Some(&focus_x) = tx_xs.get(trace.index) {
            scene.line(
                [focus_x, main_height - 6.0],
                [focus_x, panel_top],
                Stroke::dashed(FAINT, 1.0, 3.0, 4.0),
            );
        }
    }

    if show_annotations && options.show_transactions {
        let canvas = Rect::new(0.0, 0.0, width, height);
        for annotation in &doc.annotations {
            if let Some(AnnotationTarget::Transaction { index }) = &annotation.target {
                if let Some(&x) = tx_xs.get(*index) {
                    crate::render_annotation::draw_annotation(
                        &mut scene,
                        annotation,
                        None,
                        Some([x, 60.0]),
                        Some(canvas),
                    );
                }
            }
        }
    }

    TimingView {
        scene,
        width,
        height,
    }
}

struct FocusNetwork {
    scene: Scene,
    layout: crate::layout::NetworkLayout,
    bounds: Rect,
    active: HashSet<NodeId>,
    snapshots: HashSet<NodeId>,
    holds: HashSet<NodeId>,
}

fn build_focus_network(doc: &Document, trace: &TransactionTrace) -> Option<FocusNetwork> {
    let active: HashSet<NodeId> = trace
        .events
        .keys()
        .chain(trace.updates.keys())
        .copied()
        .collect();
    if active.is_empty() {
        return None;
    }
    let mut ids = active.clone();
    for id in &active {
        for edge in &doc.edges {
            if edge.from == *id {
                ids.insert(edge.to);
            }
            if edge.to == *id {
                ids.insert(edge.from);
            }
        }
    }
    if ids.len() > 18 {
        ids = active.clone();
    }
    let mut focus = doc.clone();
    focus.nodes.retain(|node| ids.contains(&node.id));
    focus
        .edges
        .retain(|edge| ids.contains(&edge.from) && ids.contains(&edge.to));
    focus.components.clear();
    focus.annotations.clear();
    for node in &mut focus.nodes {
        if let Some((_, short)) = node.name.rsplit_once('/') {
            node.name = short.to_string();
        }
    }
    if focus.nodes.is_empty() {
        return None;
    }
    crate::layout::auto_layout(&mut focus);
    let layout = crate::layout::layout(&focus);
    let scene = crate::render_network::render_network(&focus, &layout);
    let bounds = scene.bounds()?;
    let snapshots = ids
        .iter()
        .copied()
        .filter(|id| trace.events.contains_key(id))
        .filter(|id| {
            doc.node(*id)
                .map(|node| matches!(node.kind, NodeKind::Snapshot { .. }))
                .unwrap_or(false)
        })
        .collect();
    let holds = ids
        .iter()
        .copied()
        .filter(|id| trace.updates.contains_key(id))
        .filter(|id| {
            doc.node(*id)
                .map(|node| matches!(node.kind, NodeKind::Hold { .. }))
                .unwrap_or(false)
        })
        .collect();
    Some(FocusNetwork {
        scene,
        layout,
        bounds,
        active,
        snapshots,
        holds,
    })
}

fn draw_stream(
    scene: &mut Scene,
    events: &[crate::sim::SimEvent],
    x0: f32,
    x1: f32,
    baseline: f32,
    map_x: impl Fn(f64) -> f32,
    show_values: bool,
) {
    if events.is_empty() {
        scene.text(
            [x0 + 8.0, baseline - 13.0],
            "(no events)",
            11.0,
            FAINT,
            TextAnchor::Start,
        );
        return;
    }
    let mut levels: Vec<Vec<(f32, f32)>> = vec![Vec::new(); 4];
    for ev in events {
        let x = map_x(ev.time).clamp(x0, x1 - 6.0);
        if !show_values {
            scene.circle([x, baseline], 2.8, INK, None);
            continue;
        }
        let size = 12.0;
        let label = timing_label(&ev.value, 20);
        let w = text_width(&label, size) + 13.0;
        let half = w / 2.0;
        let left = x - half;
        let right = x + half;
        let mut level = 0usize;
        for (li, occupied) in levels.iter().enumerate() {
            if !occupied
                .iter()
                .any(|(a, b)| left <= *b + 3.0 && right >= *a - 3.0)
            {
                level = li;
                break;
            }
            if li == levels.len() - 1 {
                level = 0;
            }
        }
        levels[level].push((left, right));
        let box_center_y = match level {
            0 => baseline,
            1 => baseline - 22.0,
            2 => baseline + 22.0,
            _ => {
                let step = (level / 2) as f32 * 24.0;
                if level.is_multiple_of(2) {
                    baseline + 22.0 + step
                } else {
                    baseline - 22.0 - step
                }
            }
        };
        let box_rect = Rect::new(left, box_center_y - 10.0, w, 20.0);
        if level > 0 {
            let connector_y = if box_center_y < baseline {
                box_rect.bottom()
            } else {
                box_rect.y
            };
            scene.line([x, connector_y], [x, baseline], Stroke::new(INK, 1.0));
            scene.circle([x, baseline], 2.4, INK, None);
        }
        scene.rect(box_rect, 4.0, WHITE, Some(Stroke::new(INK, 1.1)));
        scene.text([x, box_center_y], label, size, INK, TextAnchor::Center);
    }
}

fn draw_cell(
    scene: &mut Scene,
    steps: &[SimStep],
    x0: f32,
    x1: f32,
    baseline: f32,
    transaction_end: impl Fn(f64) -> f32,
    show_values: bool,
) {
    if steps.is_empty() {
        scene.text(
            [x0 + 8.0, baseline - 13.0],
            "(no value)",
            11.0,
            FAINT,
            TextAnchor::Start,
        );
        return;
    }
    let band_end = (x1 - 26.0).max(x0 + 24.0);
    let top = baseline - 9.0;
    let bottom = baseline + 9.0;
    let slant = 6.0;
    scene.path(
        vec![
            [x0 + slant, top],
            [band_end, top],
            [band_end - slant, bottom],
            [x0, bottom],
        ],
        true,
        BLUE,
        Some(Stroke::new(CELL_INK, 1.2)),
    );

    let xs: Vec<f32> = steps
        .iter()
        .enumerate()
        .map(|(index, step)| {
            if index == 0 {
                x0
            } else {
                transaction_end(step.time).clamp(x0, band_end)
            }
        })
        .collect();

    for x in xs.iter().skip(1) {
        if *x <= x0 + 2.0 || *x >= band_end - 2.0 {
            continue;
        }
        scene.line([*x, top], [*x, bottom], Stroke::new(CELL_INK, 1.0));
    }

    if !show_values {
        return;
    }
    for (index, step) in steps.iter().enumerate() {
        let start = xs[index];
        let end = xs.get(index + 1).copied().unwrap_or(band_end);
        if end - start < 18.0 {
            continue;
        }
        let label = timing_label(&step.value, 22);
        let size = 11.5;
        if text_width(&label, size) + 10.0 <= end - start {
            scene.text(
                [(start + end) / 2.0, baseline],
                label,
                size,
                INK,
                TextAnchor::Center,
            );
        } else {
            scene.text(
                [(start + end) / 2.0, top - 8.0],
                fit_text(&label, end - start, 10.0),
                10.0,
                CELL_INK,
                TextAnchor::Center,
            );
        }
    }
}

fn truncate(text: &str, width: f32, size: f32) -> String {
    crate::model::fit_text(text, width, size)
}

fn timing_label(value: &crate::model::Value, max: usize) -> String {
    match value {
        crate::model::Value::Unit => "unit".into(),
        other => other.short(max),
    }
}
