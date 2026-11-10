use crate::execution::LaneSpec;
use crate::model::{
    fit_text, text_width, AnnotationTarget, Document, NodeId, NodeKind, PortType, Rect,
};
use crate::scene::{
    arrow_head, Color, Scene, Stroke, TextAnchor, ACCENT, BLUE, CELL_INK, FAINT, INK, MUTED, WHITE,
    YELLOW,
};
use crate::sim::{SimStep, Simulation, TransactionTrace};
use std::collections::{HashMap, HashSet};

const LABEL_W: f32 = 220.0;
const HEADER_H: f32 = 58.0;
const ROW_H: f32 = 66.0;
const FOOTER_H: f32 = 52.0;
const EVENT_FIRST_FRAC: f32 = 0.08;
const EVENT_LAST_FRAC: f32 = 0.76;

#[derive(Clone, Copy, Debug)]
struct EventSlot {
    x: f32,
    order: usize,
    count: usize,
    half_width: f32,
}

#[derive(Clone, Copy, Debug)]
struct LaneGeometry {
    x0: f32,
    x1: f32,
    baseline: f32,
}

struct DependencyContext<'a> {
    doc: &'a Document,
    sim: &'a Simulation,
    lanes: &'a [LaneSpec],
    event_slots: &'a HashMap<(NodeId, i64), EventSlot>,
    baselines: &'a [f32],
    tx_xs: &'a [f32],
    x1: f32,
}

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
    let mut event_slots: HashMap<(NodeId, i64), EventSlot> = HashMap::new();
    for trace in &sim.traces {
        let start = tx_xs.get(trace.index).copied().unwrap_or(x0);
        let end = tx_xs.get(trace.index + 1).copied().unwrap_or(x1);
        let count = trace.event_order.len();
        for (order, id) in trace.event_order.iter().enumerate() {
            let half_width = trace
                .events
                .get(id)
                .map(|value| (text_width(&timing_label(value, 20), 12.0) + 13.0) / 2.0)
                .unwrap_or(0.0);
            event_slots.insert(
                (*id, time_key(trace.time)),
                EventSlot {
                    x: event_slot_x(start, end, order, count),
                    order,
                    count,
                    half_width,
                },
            );
        }
    }
    let transaction_update = |time: f64| -> f32 {
        sim.transactions
            .iter()
            .position(|t| (t - time).abs() < 1e-9)
            .map(|index| {
                let start = tx_xs.get(index).copied().unwrap_or(x0);
                let end = tx_xs.get(index + 1).copied().unwrap_or(x1);
                cell_update_x(start, end)
            })
            .unwrap_or_else(|| map_x(time))
    };

    scene.text([20.0, 18.0], "transactions", 10.5, MUTED, TextAnchor::Start);
    scene.text(
        [20.0, 33.0],
        "Tn start · events in order · cell updates at Tn+1 boundary",
        9.5,
        FAINT,
        TextAnchor::Start,
    );
    scene.text(
        [x1, 18.0],
        "solid: event flow · dashed: cell dependency",
        9.0,
        FAINT,
        TextAnchor::End,
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
    let lane_baselines: Vec<f32> = (0..lanes.len())
        .map(|index| first_row_y + index as f32 * row_h + 33.0)
        .collect();

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
        let baseline = lane_baselines[index];

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
    }

    draw_dependencies(
        &mut scene,
        &DependencyContext {
            doc: focus_doc,
            sim,
            lanes,
            event_slots: &event_slots,
            baselines: &lane_baselines,
            tx_xs: &tx_xs,
            x1,
        },
    );

    for (index, lane) in lanes.iter().enumerate() {
        let baseline = lane_baselines[index];
        if lane.kind == PortType::Cell {
            draw_cell(
                &mut scene,
                sim.cell(lane.source),
                x0,
                x1,
                baseline,
                transaction_update,
                options.show_values,
            );
        } else {
            draw_stream(
                &mut scene,
                lane.source,
                sim.stream(lane.source),
                &event_slots,
                LaneGeometry { x0, x1, baseline },
                map_x,
                options.show_values,
            );
        }
    }

    draw_final_cell_states(
        &mut scene,
        lanes,
        sim,
        x1,
        &lane_baselines,
        options.show_values,
    );

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
    source: NodeId,
    events: &[crate::sim::SimEvent],
    event_slots: &HashMap<(NodeId, i64), EventSlot>,
    geometry: LaneGeometry,
    map_x: impl Fn(f64) -> f32,
    show_values: bool,
) {
    let LaneGeometry { x0, x1, baseline } = geometry;
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
        let slot = event_slots.get(&(source, time_key(ev.time))).copied();
        let x = slot
            .map(|slot| slot.x)
            .unwrap_or_else(|| map_x(ev.time))
            .clamp(x0, x1 - 6.0);
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
        if let Some(slot) = slot {
            if slot.count > 1 {
                scene.text(
                    [left + 4.0, box_rect.y - 5.0],
                    format!("#{}", slot.order + 1),
                    8.0,
                    ACCENT,
                    TextAnchor::Start,
                );
            }
        }
    }
}

fn draw_cell(
    scene: &mut Scene,
    steps: &[SimStep],
    x0: f32,
    x1: f32,
    baseline: f32,
    cell_update_x: impl Fn(f64) -> f32,
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
    let band_end = (x1 - 12.0).max(x0 + 24.0);
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
                cell_update_x(step.time).clamp(x0, band_end)
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

fn draw_final_cell_states(
    scene: &mut Scene,
    lanes: &[LaneSpec],
    sim: &Simulation,
    x1: f32,
    baselines: &[f32],
    show_values: bool,
) {
    if !show_values {
        return;
    }
    let mut has_final = false;
    for (index, lane) in lanes.iter().enumerate() {
        if lane.kind != PortType::Cell {
            continue;
        }
        let Some(last) = sim.cell(lane.source).last() else {
            continue;
        };
        let Some(transaction) = sim
            .transactions
            .iter()
            .position(|time| (time - last.time).abs() < 1e-9)
        else {
            continue;
        };
        if transaction + 1 < sim.transactions.len() {
            continue;
        }
        let baseline = baselines[index];
        let label = timing_label(&last.value, 22);
        let size = 11.5;
        let width = (text_width(&label, size) + 12.0).clamp(26.0, 38.0);
        let x = x1 + 4.0;
        scene.line([x1, baseline], [x, baseline], Stroke::new(CELL_INK, 1.0));
        scene.rect(
            Rect::new(x, baseline - 9.0, width, 18.0),
            4.0,
            BLUE,
            Some(Stroke::new(CELL_INK, 1.2)),
        );
        scene.text(
            [x + width / 2.0, baseline],
            fit_text(&label, width - 8.0, size),
            size,
            INK,
            TextAnchor::Center,
        );
        has_final = true;
    }
    if has_final {
        scene.text([x1 + 4.0, 48.0], "after", 8.5, FAINT, TextAnchor::Start);
    }
}

fn draw_dependencies(scene: &mut Scene, context: &DependencyContext<'_>) {
    let doc = context.doc;
    let sim = context.sim;
    let lanes = context.lanes;
    let event_slots = context.event_slots;
    let baselines = context.baselines;
    let tx_xs = context.tx_xs;
    let x1 = context.x1;
    let mut lane_by_id: HashMap<NodeId, usize> = HashMap::new();
    for (index, lane) in lanes.iter().enumerate() {
        lane_by_id.entry(lane.source).or_insert(index);
    }
    let event_color = crate::scene::Color(45, 108, 223, 145);
    let cell_color = crate::scene::Color(74, 121, 158, 125);

    for trace in &sim.traces {
        let key = time_key(trace.time);
        let start = tx_xs.get(trace.index).copied().unwrap_or(0.0);
        let boundary = tx_xs.get(trace.index + 1).copied().unwrap_or(x1);
        let updates: HashMap<NodeId, usize> = trace
            .update_order
            .iter()
            .enumerate()
            .map(|(index, id)| (*id, index))
            .collect();

        for edge in &doc.edges {
            let Some(&source_lane) = lane_by_id.get(&edge.from) else {
                continue;
            };
            let Some(&target_lane) = lane_by_id.get(&edge.to) else {
                continue;
            };
            let source_event = event_slots.get(&(edge.from, key)).copied();
            let target_event = event_slots.get(&(edge.to, key)).copied();
            let source_is_cell = lanes[source_lane].kind == PortType::Cell;
            let target_is_cell = lanes[target_lane].kind == PortType::Cell;
            let target_updated = updates.contains_key(&edge.to);
            let source_updated = updates.get(&edge.from).copied();

            let (source, target, is_cell_dependency) = match (source_event, target_event) {
                (Some(source), Some(target)) => (
                    event_anchor(source, baselines[source_lane]),
                    event_target(target, baselines[target_lane]),
                    false,
                ),
                (Some(source), None) if target_is_cell && target_updated => (
                    event_anchor(source, baselines[source_lane]),
                    [boundary - 2.0, baselines[target_lane] - 12.0],
                    true,
                ),
                (None, Some(target)) if source_is_cell => {
                    let source_x = start;
                    (
                        [source_x, baselines[source_lane] - 12.0],
                        event_target(target, baselines[target_lane]),
                        true,
                    )
                }
                (None, None) if source_is_cell && target_is_cell && target_updated => {
                    let source_x = source_updated
                        .filter(|source_order| {
                            updates
                                .get(&edge.to)
                                .is_some_and(|target_order| source_order < target_order)
                        })
                        .map(|_| boundary - 8.0)
                        .unwrap_or(start);
                    (
                        [source_x, baselines[source_lane] - 12.0],
                        [boundary - 2.0, baselines[target_lane] - 12.0],
                        true,
                    )
                }
                _ => continue,
            };

            if target[0] <= source[0] + 3.0 {
                continue;
            }
            let middle = ((source[0] + target[0]) / 2.0)
                .max(source[0] + 6.0)
                .min(target[0] - 6.0);
            let points = vec![source, [middle, source[1]], [middle, target[1]], target];
            let color = if is_cell_dependency {
                cell_color
            } else {
                event_color
            };
            let stroke = if is_cell_dependency {
                Stroke::dashed(color, 0.9, 2.5, 2.5)
            } else {
                Stroke::new(color, 1.1)
            };
            scene.polyline(crate::scene::rounded_polyline(&points, 5.0), stroke);
            let head = arrow_head(target, [1.0, 0.0], 5.0);
            scene.path(head, true, color, None);
        }
    }
}

fn event_anchor(slot: EventSlot, baseline: f32) -> [f32; 2] {
    [slot.x + slot.half_width + 2.0, baseline]
}

fn event_target(slot: EventSlot, baseline: f32) -> [f32; 2] {
    [slot.x - slot.half_width - 2.0, baseline]
}

fn event_slot_x(start: f32, end: f32, order: usize, count: usize) -> f32 {
    let span = (end - start).max(1.0);
    let fraction = if count <= 1 {
        EVENT_FIRST_FRAC
    } else {
        EVENT_FIRST_FRAC + (EVENT_LAST_FRAC - EVENT_FIRST_FRAC) * order as f32 / (count - 1) as f32
    };
    start + span * fraction
}

fn cell_update_x(start: f32, end: f32) -> f32 {
    end.max(start + 1.0)
}

fn time_key(time: f64) -> i64 {
    (time * 1_000_000.0).round() as i64
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_slots_follow_the_causal_order() {
        let xs: Vec<f32> = (0..3)
            .map(|order| event_slot_x(100.0, 200.0, order, 3))
            .collect();
        assert!(xs.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(xs[0] > 100.0);
        assert!(xs[2] < cell_update_x(100.0, 200.0));
    }

    #[test]
    fn cell_updates_happen_after_the_event_sequence() {
        let update = cell_update_x(100.0, 200.0);
        assert!(update > event_slot_x(100.0, 200.0, 9, 10));
        assert_eq!(update, 200.0);
    }
}
