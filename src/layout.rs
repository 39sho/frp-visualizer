use crate::model::{text_width, Document, Edge, NodeId, NodeKind, Rect};
use crate::scene::cloud_points;
use std::cell::{Cell, RefCell};
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RouteStyle {
    Hybrid,
    #[default]
    Orthogonal,
}

#[derive(Clone, Debug, Default)]
pub struct LayoutOptions {
    pub route_style: RouteStyle,
}

#[derive(Clone, Debug)]
pub struct EdgeRoute {
    pub points: Vec<[f32; 2]>,
    pub label_pos: [f32; 2],
    pub feedback: bool,
    /// Point where this edge leaves the shared fan-out trunk.
    pub fork: Option<[f32; 2]>,
    /// Interior branch point that should be drawn as a junction marker.
    pub junction: Option<[f32; 2]>,
}

#[derive(Clone, Debug, Default)]
pub struct NetworkLayout {
    pub node_rects: HashMap<NodeId, Rect>,
    pub edge_routes: HashMap<u64, EdgeRoute>,
    pub bounds: Rect,
}

impl NetworkLayout {
    pub fn input_port(&self, doc: &Document, node: NodeId, port: usize) -> Option<[f32; 2]> {
        let rect = *self.node_rects.get(&node)?;
        let node = doc.node(node)?;
        if port >= node.kind.input_types().len() {
            return None;
        }
        Some(input_port_pos(node, rect, port))
    }
}

pub fn layout(doc: &Document) -> NetworkLayout {
    layout_with(doc, &LayoutOptions::default())
}

pub fn layout_with(doc: &Document, options: &LayoutOptions) -> NetworkLayout {
    let mut node_rects = HashMap::new();
    for node in &doc.nodes {
        node_rects.insert(node.id, node.rect());
    }

    let branches = fanout_branches(doc, &node_rects);
    let mut routes = HashMap::new();
    let mut routed: Vec<Vec<[f32; 2]>> = Vec::new();
    let mut routed_groups: Vec<Option<FanoutKey>> = Vec::new();
    for edge in &doc.edges {
        let (Some(from), Some(to)) = (doc.node(edge.from), doc.node(edge.to)) else {
            continue;
        };
        let Some(from_rect) = node_rects.get(&edge.from).copied() else {
            continue;
        };
        let Some(to_rect) = node_rects.get(&edge.to).copied() else {
            continue;
        };
        let start = output_port_pos(from, from_rect, edge.from_port);
        let end = input_port_pos(to, to_rect, edge.to_port);
        let end_dir = input_port_dir(to, edge.to_port);
        let obstacles = route_obstacles(doc, &node_rects, edge.from, edge.to);
        let branch = branches.get(&edge.id);
        let group = branch.map(|branch| branch.key);
        let mut existing = Vec::new();
        for (points, source) in routed.iter().zip(routed_groups.iter()) {
            if group.is_some() && *source == group {
                continue;
            }
            existing.push(points.clone());
        }
        let route_start = branch.map(|branch| branch.fork).unwrap_or(start);
        let middle = if options.route_style == RouteStyle::Orthogonal {
            route_edge_orthogonal(&obstacles, &existing, route_start, [1.0, 0.0], end, end_dir)
                .unwrap_or_else(|| {
                    route_edge(&obstacles, &existing, route_start, [1.0, 0.0], end, end_dir)
                })
        } else {
            route_edge(&obstacles, &existing, route_start, [1.0, 0.0], end, end_dir)
        };
        // Keep labels off the shared trunk so fan-out branches stay readable.
        let label_pos = route_label_pos(&middle);
        let mut points = if let Some(branch) = branch {
            let mut points = branch.prefix.clone();
            points.extend(middle.iter().skip(1).copied());
            points
        } else {
            middle
        };
        dedup_points(&mut points);
        routed.push(points.clone());
        routed_groups.push(group);
        routes.insert(
            edge.id,
            EdgeRoute {
                points,
                label_pos,
                feedback: end[0] < start[0] || end[1] < start[1] - 20.0,
                fork: branch.map(|branch| branch.fork),
                junction: branch.and_then(|branch| branch.junction),
            },
        );
    }

    add_crossing_hops(&mut routes);

    let mut layout = NetworkLayout {
        node_rects,
        edge_routes: routes,
        bounds: Rect::default(),
    };
    layout.bounds = compute_bounds(doc, &layout);
    layout
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct FanoutKey {
    node: NodeId,
    port: usize,
}

#[derive(Clone, Debug)]
struct FanoutBranch {
    key: FanoutKey,
    fork: [f32; 2],
    prefix: Vec<[f32; 2]>,
    junction: Option<[f32; 2]>,
}

fn fanout_branches(
    doc: &Document,
    node_rects: &HashMap<NodeId, Rect>,
) -> HashMap<u64, FanoutBranch> {
    let mut groups: HashMap<FanoutKey, Vec<&Edge>> = HashMap::new();
    for edge in &doc.edges {
        let (Some(from), Some(to)) = (doc.node(edge.from), doc.node(edge.to)) else {
            continue;
        };
        if edge.from_port >= from.kind.output_types().len()
            || edge.to_port >= to.kind.input_types().len()
            || !node_rects.contains_key(&edge.from)
            || !node_rects.contains_key(&edge.to)
        {
            continue;
        }
        groups
            .entry(FanoutKey {
                node: edge.from,
                port: edge.from_port,
            })
            .or_default()
            .push(edge);
    }

    let mut branches = HashMap::new();
    for (key, edges) in groups {
        // Two wires can already split cleanly at the source: one goes above,
        // one goes below, or their targets are far enough apart. Sequential
        // routing only starts weaving once a port feeds three or more inputs.
        if edges.len() < 3 {
            continue;
        }
        let (Some(from), Some(from_rect)) =
            (doc.node(key.node), node_rects.get(&key.node).copied())
        else {
            continue;
        };
        let port = output_port_pos(from, from_rect, key.port);

        let mut targets: Vec<(u64, f32, f32)> = edges
            .iter()
            .filter_map(|edge| {
                let to = doc.node(edge.to)?;
                let rect = node_rects.get(&edge.to)?;
                let y = input_port_pos(to, *rect, edge.to_port)[1];
                Some((edge.id, y, rect.x))
            })
            .collect();
        targets.sort_by(|a, b| {
            a.1.partial_cmp(&b.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal))
                .then_with(|| a.0.cmp(&b.0))
        });

        let desired: Vec<f32> = targets.iter().map(|(_, y, _)| *y).collect();
        let lanes = fanout_lanes(&desired);
        let obstacles =
            route_obstacles_excluding(doc, node_rects, key.node, key.node, Some(key.node));
        let base_x = from_rect.right() + FANOUT_STEM;
        let Some((fork_x, detour_y)) = fanout_trunk(port, &lanes, base_x, &obstacles) else {
            // No safe trunk: leave this port to the normal router instead of
            // drawing a bus through an unrelated node.
            continue;
        };
        let trunk_min = lanes.iter().copied().fold(detour_y, f32::min);
        let trunk_max = lanes.iter().copied().fold(detour_y, f32::max);

        for ((edge_id, _, _), lane) in targets.iter().zip(lanes.iter()) {
            let fork = [fork_x, *lane];
            let mut prefix = vec![port];
            if (detour_y - port[1]).abs() > 0.5 {
                prefix.push([port[0], detour_y]);
                prefix.push([fork_x, detour_y]);
            } else {
                prefix.push([fork_x, port[1]]);
            }
            prefix.push(fork);
            dedup_points(&mut prefix);
            let interior = *lane > trunk_min + 0.5 && *lane < trunk_max - 0.5;
            let junction = if interior { Some(fork) } else { None };
            branches.insert(
                *edge_id,
                FanoutBranch {
                    key,
                    fork,
                    prefix,
                    junction,
                },
            );
        }
    }
    branches
}

fn fanout_trunk(
    port: [f32; 2],
    lanes: &[f32],
    base_x: f32,
    obstacles: &[Rect],
) -> Option<(f32, f32)> {
    let min_lane = lanes.iter().copied().fold(port[1], f32::min);
    let max_lane = lanes.iter().copied().fold(port[1], f32::max);

    // Prefer a straight horizontal stem at the output-port height.
    for step in 0..8 {
        let x = base_x + step as f32 * 8.0;
        if segment_clear([port[0], port[1]], [x, port[1]], obstacles)
            && segment_clear([x, min_lane], [x, max_lane], obstacles)
            && fanout_stubs_clear(x, lanes, obstacles)
        {
            return Some((x, port[1]));
        }
    }

    // If the direct stem is blocked, rise or fall beside the source first,
    // then run the trunk above/below the obstruction.
    for step in 0..48 {
        let x = base_x + step as f32 * 8.0;
        if !segment_clear([x, min_lane], [x, max_lane], obstacles)
            || !fanout_stubs_clear(x, lanes, obstacles)
        {
            continue;
        }
        let mut candidates = vec![port[1]];
        candidates.extend(lanes.iter().copied());
        for rect in obstacles {
            if rect.right() > port[0] && rect.x < x {
                candidates.push(rect.y - 2.0);
                candidates.push(rect.bottom() + 2.0);
            }
        }
        candidates.sort_by(|a, b| {
            (a - port[1])
                .abs()
                .partial_cmp(&(b - port[1]).abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        candidates.dedup_by(|a, b| (*a - *b).abs() < 0.5);
        for y in candidates {
            if !segment_clear([port[0], port[1]], [port[0], y], obstacles) {
                continue;
            }
            if !segment_clear([port[0], y], [x, y], obstacles) {
                continue;
            }
            let bus_min = y.min(min_lane);
            let bus_max = y.max(max_lane);
            if segment_clear([x, bus_min], [x, bus_max], obstacles) {
                return Some((x, y));
            }
        }
    }
    None
}

fn fanout_stubs_clear(fork_x: f32, lanes: &[f32], obstacles: &[Rect]) -> bool {
    const START_STUB: f32 = 14.0;
    lanes
        .iter()
        .all(|lane| segment_clear([fork_x, *lane], [fork_x + START_STUB, *lane], obstacles))
}

fn fanout_lanes(desired: &[f32]) -> Vec<f32> {
    if desired.len() <= 1 {
        return desired.to_vec();
    }
    let mut lanes = desired.to_vec();
    for index in 1..lanes.len() {
        lanes[index] = lanes[index].max(lanes[index - 1] + FANOUT_LANE_GAP);
    }
    let desired_center = desired.iter().sum::<f32>() / desired.len() as f32;
    let lane_center = lanes.iter().sum::<f32>() / lanes.len() as f32;
    let shift = desired_center - lane_center;
    for lane in &mut lanes {
        *lane += shift;
    }
    lanes
}

fn input_port_dir(node: &crate::model::Node, port: usize) -> [f32; 2] {
    if matches!(node.kind, NodeKind::Snapshot { .. }) && port == 1 {
        [0.0, 1.0]
    } else if matches!(node.kind, NodeKind::Hold { .. }) && port == 0 {
        [0.0, -1.0]
    } else {
        [-1.0, 0.0]
    }
}

const ROUTE_CLEARANCE: f32 = 8.0;
const LABEL_CLEARANCE: f32 = 20.0;
const EDGE_CLEARANCE: f32 = 12.0;
const SHARED_RADIUS: f32 = 22.0;
const FANOUT_STEM: f32 = 34.0;
const FANOUT_LANE_GAP: f32 = EDGE_CLEARANCE + 2.0;

const SEGMENT_INDEX_CELL: f32 = 64.0;

struct SegmentIndex {
    segments: Vec<([f32; 2], [f32; 2])>,
    buckets: HashMap<(i32, i32), Vec<usize>>,
    seen: RefCell<Vec<u64>>,
    generation: Cell<u64>,
}

impl SegmentIndex {
    fn new(existing: &[Vec<[f32; 2]>]) -> Self {
        let mut segments = Vec::new();
        let mut buckets: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
        let margin = EDGE_CLEARANCE + 2.0;
        for route in existing {
            for window in route.windows(2) {
                let a = window[0];
                let b = window[1];
                let id = segments.len();
                segments.push((a, b));
                let min_x = a[0].min(b[0]) - margin;
                let max_x = a[0].max(b[0]) + margin;
                let min_y = a[1].min(b[1]) - margin;
                let max_y = a[1].max(b[1]) + margin;
                let x0 = (min_x / SEGMENT_INDEX_CELL).floor() as i32;
                let x1 = (max_x / SEGMENT_INDEX_CELL).floor() as i32;
                let y0 = (min_y / SEGMENT_INDEX_CELL).floor() as i32;
                let y1 = (max_y / SEGMENT_INDEX_CELL).floor() as i32;
                for x in x0..=x1 {
                    for y in y0..=y1 {
                        buckets.entry((x, y)).or_default().push(id);
                    }
                }
            }
        }
        let seen = RefCell::new(vec![0; segments.len()]);
        Self {
            segments,
            buckets,
            seen,
            generation: Cell::new(0),
        }
    }

    fn nearby_into(&self, a: [f32; 2], b: [f32; 2], ids: &mut Vec<usize>) {
        ids.clear();
        let generation = self.generation.get().wrapping_add(1);
        self.generation.set(generation);
        let mut seen = self.seen.borrow_mut();
        if seen.len() < self.segments.len() {
            seen.resize(self.segments.len(), 0);
        }
        let x0 = (a[0].min(b[0]) / SEGMENT_INDEX_CELL).floor() as i32;
        let x1 = (a[0].max(b[0]) / SEGMENT_INDEX_CELL).floor() as i32;
        let y0 = (a[1].min(b[1]) / SEGMENT_INDEX_CELL).floor() as i32;
        let y1 = (a[1].max(b[1]) / SEGMENT_INDEX_CELL).floor() as i32;
        for x in x0..=x1 {
            for y in y0..=y1 {
                if let Some(bucket) = self.buckets.get(&(x, y)) {
                    for id in bucket {
                        if seen[*id] != generation {
                            seen[*id] = generation;
                            ids.push(*id);
                        }
                    }
                }
            }
        }
    }

    fn nearby(&self, a: [f32; 2], b: [f32; 2]) -> Vec<usize> {
        let mut ids = Vec::new();
        self.nearby_into(a, b, &mut ids);
        ids
    }
}

fn parallel_conflict_indexed(
    a: [f32; 2],
    b: [f32; 2],
    index: &SegmentIndex,
    edge_start: [f32; 2],
    edge_end: [f32; 2],
) -> bool {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let ab_length = (ab[0] * ab[0] + ab[1] * ab[1]).sqrt();
    if ab_length < 1e-3 {
        return false;
    }
    for id in index.nearby(a, b) {
        let (c, d) = index.segments[id];
        let Some((c, d)) = trim_segment(c, d, edge_start, SHARED_RADIUS) else {
            continue;
        };
        let Some((c, d)) = trim_segment(c, d, edge_end, SHARED_RADIUS) else {
            continue;
        };
        let cd = [d[0] - c[0], d[1] - c[1]];
        let cd_length = (cd[0] * cd[0] + cd[1] * cd[1]).sqrt();
        if cd_length < 1e-3 {
            continue;
        }
        let sin = (ab[0] * cd[1] - ab[1] * cd[0]).abs() / (ab_length * cd_length);
        if sin < 0.25 && segment_distance(a, b, c, d) < EDGE_CLEARANCE {
            return true;
        }
    }
    false
}

fn crossing_penalty_indexed(
    a: [f32; 2],
    b: [f32; 2],
    index: &SegmentIndex,
    edge_start: [f32; 2],
    edge_end: [f32; 2],
) -> f32 {
    let mut crossings = 0.0;
    for id in index.nearby(a, b) {
        let (c, d) = index.segments[id];
        let Some((c, d)) = trim_segment(c, d, edge_start, SHARED_RADIUS) else {
            continue;
        };
        let Some((c, d)) = trim_segment(c, d, edge_end, SHARED_RADIUS) else {
            continue;
        };
        if segment_crossing(a, b, c, d).is_some() {
            crossings += CROSSING_PENALTY;
        }
    }
    crossings
}

fn segment_ok_indexed(
    a: [f32; 2],
    b: [f32; 2],
    obstacles: &[Rect],
    index: &SegmentIndex,
    edge_start: [f32; 2],
    edge_end: [f32; 2],
    avoidance: EdgeAvoidance,
) -> bool {
    if !segment_clear(a, b, obstacles) {
        return false;
    }
    if avoidance == EdgeAvoidance::Hard
        && parallel_conflict_indexed(a, b, index, edge_start, edge_end)
    {
        return false;
    }
    true
}

fn route_obstacles(
    doc: &Document,
    node_rects: &HashMap<NodeId, Rect>,
    start_node: NodeId,
    end_node: NodeId,
) -> Vec<Rect> {
    route_obstacles_excluding(doc, node_rects, start_node, end_node, None)
}

fn route_obstacles_excluding(
    doc: &Document,
    node_rects: &HashMap<NodeId, Rect>,
    start_node: NodeId,
    end_node: NodeId,
    exclude: Option<NodeId>,
) -> Vec<Rect> {
    let mut obstacles = Vec::new();
    for (id, rect) in node_rects {
        if exclude == Some(*id) {
            continue;
        }
        if *id == start_node || *id == end_node {
            obstacles.push(rect.expand(ROUTE_CLEARANCE));
            if let Some(label) = doc.node(*id).and_then(|node| node_label_rect(node, *rect)) {
                obstacles.push(label.expand(2.0));
            }
        } else if doc
            .node(*id)
            .map(|node| node.kind.is_component())
            .unwrap_or(false)
        {
            // Cloud lobes bulge past the node rectangle. Expand the obstacle
            // only as far as the nearest neighbour allows, so tight manual
            // layouts keep a routing corridor instead of falling back to a
            // straight line through the cloud.
            let [left, right, top, bottom] = component_cloud_padding(*rect, node_rects, *id);
            obstacles.push(Rect::new(
                rect.x - ROUTE_CLEARANCE - left,
                rect.y - ROUTE_CLEARANCE - top,
                rect.w + ROUTE_CLEARANCE * 2.0 + left + right,
                rect.h + ROUTE_CLEARANCE * 2.0 + top + bottom,
            ));
        } else {
            obstacles.push(Rect::new(
                rect.x - ROUTE_CLEARANCE,
                rect.y - ROUTE_CLEARANCE - LABEL_CLEARANCE,
                rect.w + ROUTE_CLEARANCE * 2.0,
                rect.h + ROUTE_CLEARANCE * 2.0 + LABEL_CLEARANCE,
            ));
        }
    }
    obstacles
}

fn component_cloud_overhang(rect: Rect) -> [f32; 2] {
    let mut overhang_x: f32 = 0.0;
    let mut overhang_y: f32 = 0.0;
    for point in cloud_points(rect) {
        overhang_x = overhang_x.max((rect.x - point[0]).max(point[0] - rect.right()));
        overhang_y = overhang_y.max((rect.y - point[1]).max(point[1] - rect.bottom()));
    }
    [overhang_x, overhang_y]
}

fn component_cloud_padding(rect: Rect, node_rects: &HashMap<NodeId, Rect>, id: NodeId) -> [f32; 4] {
    let [overhang_x, overhang_y] = component_cloud_overhang(rect);
    let mut left = overhang_x;
    let mut right = overhang_x;
    let mut top = overhang_y;
    let mut bottom = overhang_y;
    // Leave room for the router's 14px endpoint stub plus a small gap when
    // another node sits next to the component.
    let side_corridor = ROUTE_CLEARANCE * 2.0 + 16.0;
    let vertical_corridor = side_corridor + LABEL_CLEARANCE;
    for (other_id, other) in node_rects {
        if *other_id == id {
            continue;
        }
        // Only nodes that actually share a side corridor should cap the
        // expansion. A far-away node must not shrink clearance around the
        // cloud just because it happens to be above or below it.
        let overlaps_y = other.bottom() > rect.y && other.y < rect.bottom();
        let overlaps_x = other.right() > rect.x && other.x < rect.right();
        if overlaps_y {
            if other.right() <= rect.x {
                left = left.min((rect.x - other.right() - side_corridor).max(0.0));
            }
            if other.x >= rect.right() {
                right = right.min((other.x - rect.right() - side_corridor).max(0.0));
            }
        }
        if overlaps_x {
            if other.bottom() <= rect.y {
                top = top.min((rect.y - other.bottom() - side_corridor).max(0.0));
            }
            if other.y >= rect.bottom() {
                // The node below draws its label above itself.
                bottom = bottom.min((other.y - rect.bottom() - vertical_corridor).max(0.0));
            }
        }
    }
    [left, right, top, bottom]
}

fn node_label_rect(node: &crate::model::Node, rect: Rect) -> Option<Rect> {
    let (text, center_x) = match &node.kind {
        NodeKind::Hold { .. } => {
            let width = text_width("hold", 13.5);
            ("hold", rect.x + 12.0 + width / 2.0)
        }
        NodeKind::Map { .. }
        | NodeKind::Filter { .. }
        | NodeKind::Merge
        | NodeKind::Snapshot { .. }
        | NodeKind::Lift { .. }
        | NodeKind::Never => (node.kind.primitive(), rect.center()[0]),
        _ => return None,
    };
    let width = text_width(text, 13.5);
    Some(Rect::new(
        center_x - width / 2.0,
        rect.y - 24.0,
        width,
        18.0,
    ))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EdgeAvoidance {
    Hard,
    Soft,
    Off,
}

fn route_edge(
    obstacles: &[Rect],
    existing: &[Vec<[f32; 2]>],
    start: [f32; 2],
    start_dir: [f32; 2],
    end: [f32; 2],
    end_dir: [f32; 2],
) -> Vec<[f32; 2]> {
    let distance = ((end[0] - start[0]).powi(2) + (end[1] - start[1]).powi(2)).sqrt();
    if distance < 2.0 * (ROUTE_CLEARANCE + 2.0) {
        return vec![start, end];
    }
    let stub = (distance * 0.25).clamp(ROUTE_CLEARANCE + 2.0, 14.0);
    let start_stub = [
        start[0] + start_dir[0] * stub,
        start[1] + start_dir[1] * stub,
    ];
    let end_stub = [end[0] + end_dir[0] * stub, end[1] + end_dir[1] * stub];

    let middle = route_between(
        start_stub,
        end_stub,
        obstacles,
        existing,
        start,
        end,
        EdgeAvoidance::Hard,
    )
    .or_else(|| {
        route_between(
            start_stub,
            end_stub,
            obstacles,
            existing,
            start,
            end,
            EdgeAvoidance::Soft,
        )
    })
    .or_else(|| {
        route_between(
            start_stub,
            end_stub,
            obstacles,
            existing,
            start,
            end,
            EdgeAvoidance::Off,
        )
    })
    .unwrap_or_else(|| vec![start_stub, end_stub]);

    let mut points = Vec::with_capacity(middle.len() + 2);
    points.push(start);
    points.extend(middle);
    points.push(end);
    dedup_points(&mut points);
    points
}

fn route_edge_orthogonal(
    obstacles: &[Rect],
    existing: &[Vec<[f32; 2]>],
    start: [f32; 2],
    start_dir: [f32; 2],
    end: [f32; 2],
    end_dir: [f32; 2],
) -> Option<Vec<[f32; 2]>> {
    let distance = ((end[0] - start[0]).powi(2) + (end[1] - start[1]).powi(2)).sqrt();
    if distance < 2.0 * (ROUTE_CLEARANCE + 2.0) {
        return Some(vec![start, end]);
    }
    let stub = (distance * 0.25).clamp(ROUTE_CLEARANCE + 2.0, 14.0);
    let start_stub = [
        start[0] + start_dir[0] * stub,
        start[1] + start_dir[1] * stub,
    ];
    let end_stub = [end[0] + end_dir[0] * stub, end[1] + end_dir[1] * stub];
    let index = SegmentIndex::new(existing);
    let middle = orthogonal_route(
        start_stub,
        end_stub,
        obstacles,
        existing,
        &index,
        start,
        end,
        EdgeAvoidance::Hard,
    )
    .or_else(|| {
        orthogonal_route(
            start_stub,
            end_stub,
            obstacles,
            existing,
            &index,
            start,
            end,
            EdgeAvoidance::Soft,
        )
    })
    .or_else(|| {
        orthogonal_route(
            start_stub,
            end_stub,
            obstacles,
            existing,
            &index,
            start,
            end,
            EdgeAvoidance::Off,
        )
    })?;

    let mut points = Vec::with_capacity(middle.len() + 2);
    points.push(start);
    points.extend(middle);
    points.push(end);
    dedup_points(&mut points);
    Some(points)
}

#[allow(clippy::too_many_arguments)]
fn orthogonal_route(
    start: [f32; 2],
    end: [f32; 2],
    obstacles: &[Rect],
    existing: &[Vec<[f32; 2]>],
    index: &SegmentIndex,
    edge_start: [f32; 2],
    edge_end: [f32; 2],
    avoidance: EdgeAvoidance,
) -> Option<Vec<[f32; 2]>> {
    let mut xs = vec![start[0], end[0]];
    let mut ys = vec![start[1], end[1]];
    for rect in obstacles {
        xs.push(rect.x);
        xs.push(rect.right());
        ys.push(rect.y);
        ys.push(rect.bottom());
    }
    for route in existing {
        for point in route {
            xs.push(point[0]);
            ys.push(point[1]);
        }
        for window in route.windows(2) {
            let a = window[0];
            let b = window[1];
            if (a[1] - b[1]).abs() < 0.5 {
                ys.push(a[1]);
                ys.push(a[1] + EDGE_CLEARANCE + 1.0);
                ys.push(a[1] - EDGE_CLEARANCE - 1.0);
            } else if (a[0] - b[0]).abs() < 0.5 {
                xs.push(a[0]);
                xs.push(a[0] + EDGE_CLEARANCE + 1.0);
                xs.push(a[0] - EDGE_CLEARANCE - 1.0);
            }
        }
    }
    let xs = sorted_unique(xs);
    let ys = sorted_unique(ys);
    let nx = xs.len();
    let ny = ys.len();
    let start_x = index_of(&xs, start[0])?;
    let start_y = index_of(&ys, start[1])?;
    let end_x = index_of(&xs, end[0])?;
    let end_y = index_of(&ys, end[1])?;

    let mut point_ok = vec![true; nx * ny];
    for yi in 0..ny {
        for xi in 0..nx {
            let point = [xs[xi], ys[yi]];
            point_ok[yi * nx + xi] = !obstacles.iter().any(|rect| inside_rect_strict(point, rect));
        }
    }

    let mut horizontal = vec![None; ny * (nx.saturating_sub(1))];
    for yi in 0..ny {
        for xi in 0..nx.saturating_sub(1) {
            if !point_ok[yi * nx + xi] || !point_ok[yi * nx + xi + 1] {
                continue;
            }
            horizontal[yi * (nx - 1) + xi] = orthogonal_penalty(
                [xs[xi], ys[yi]],
                [xs[xi + 1], ys[yi]],
                obstacles,
                index,
                edge_start,
                edge_end,
                avoidance,
            );
        }
    }
    let mut vertical = vec![None; ny.saturating_sub(1) * nx];
    for yi in 0..ny.saturating_sub(1) {
        for xi in 0..nx {
            if !point_ok[yi * nx + xi] || !point_ok[(yi + 1) * nx + xi] {
                continue;
            }
            vertical[yi * nx + xi] = orthogonal_penalty(
                [xs[xi], ys[yi]],
                [xs[xi], ys[yi + 1]],
                obstacles,
                index,
                edge_start,
                edge_end,
                avoidance,
            );
        }
    }

    let state_count = nx * ny * 2;
    let mut dist = vec![f32::INFINITY; state_count];
    let mut previous = vec![usize::MAX; state_count];
    let mut heap: BinaryHeap<Reverse<(i64, usize)>> = BinaryHeap::new();
    let start_point = start_y * nx + start_x;
    for direction in 0..2 {
        let state = start_point * 2 + direction;
        dist[state] = 0.0;
        heap.push(Reverse((0, state)));
    }

    while let Some(Reverse((stored_cost, state))) = heap.pop() {
        let cost = stored_cost as f32 / 10.0;
        if cost > dist[state] + 1e-3 {
            continue;
        }
        let point = state / 2;
        let direction = state % 2;
        let xi = point % nx;
        let yi = point / nx;

        if xi > 0 {
            if let Some(penalty) = horizontal[yi * (nx - 1) + xi - 1] {
                let next_point = yi * nx + xi - 1;
                let step = xs[xi] - xs[xi - 1];
                let mut next_cost = dist[state] + step + penalty;
                if direction != 0 {
                    next_cost += BEND_PENALTY;
                }
                relax(
                    next_point * 2,
                    next_cost,
                    state,
                    &mut dist,
                    &mut previous,
                    &mut heap,
                );
            }
        }
        if xi + 1 < nx {
            if let Some(penalty) = horizontal[yi * (nx - 1) + xi] {
                let next_point = yi * nx + xi + 1;
                let step = xs[xi + 1] - xs[xi];
                let mut next_cost = dist[state] + step + penalty;
                if direction != 0 {
                    next_cost += BEND_PENALTY;
                }
                relax(
                    next_point * 2,
                    next_cost,
                    state,
                    &mut dist,
                    &mut previous,
                    &mut heap,
                );
            }
        }
        if yi > 0 {
            if let Some(penalty) = vertical[(yi - 1) * nx + xi] {
                let next_point = (yi - 1) * nx + xi;
                let step = ys[yi] - ys[yi - 1];
                let mut next_cost = dist[state] + step + penalty;
                if direction != 1 {
                    next_cost += BEND_PENALTY;
                }
                relax(
                    next_point * 2 + 1,
                    next_cost,
                    state,
                    &mut dist,
                    &mut previous,
                    &mut heap,
                );
            }
        }
        if yi + 1 < ny {
            if let Some(penalty) = vertical[yi * nx + xi] {
                let next_point = (yi + 1) * nx + xi;
                let step = ys[yi + 1] - ys[yi];
                let mut next_cost = dist[state] + step + penalty;
                if direction != 1 {
                    next_cost += BEND_PENALTY;
                }
                relax(
                    next_point * 2 + 1,
                    next_cost,
                    state,
                    &mut dist,
                    &mut previous,
                    &mut heap,
                );
            }
        }
    }

    let end_point = end_y * nx + end_x;
    let expected_direction = if (end[1] - start[1]).abs() > (end[0] - start[0]).abs() {
        1
    } else {
        0
    };
    let mut best_state = None;
    let mut best_cost = f32::INFINITY;
    for direction in 0..2 {
        let state = end_point * 2 + direction;
        if !dist[state].is_finite() {
            continue;
        }
        let cost = dist[state]
            + if direction != expected_direction {
                BEND_PENALTY
            } else {
                0.0
            };
        if cost < best_cost {
            best_cost = cost;
            best_state = Some(state);
        }
    }
    let mut state = best_state?;
    let mut indices = Vec::new();
    loop {
        indices.push(state / 2);
        if state / 2 == start_point {
            break;
        }
        let previous_state = previous[state];
        if previous_state == usize::MAX {
            return None;
        }
        state = previous_state;
    }
    indices.reverse();
    let mut path: Vec<[f32; 2]> = indices
        .iter()
        .map(|index| [xs[index % nx], ys[index / nx]])
        .collect();
    dedup_points(&mut path);
    simplify_orthogonal(&mut path, obstacles, index, edge_start, edge_end, avoidance);
    Some(path)
}

const BEND_PENALTY: f32 = 30.0;
const CROSSING_PENALTY: f32 = 240.0;

fn orthogonal_penalty(
    a: [f32; 2],
    b: [f32; 2],
    obstacles: &[Rect],
    index: &SegmentIndex,
    edge_start: [f32; 2],
    edge_end: [f32; 2],
    avoidance: EdgeAvoidance,
) -> Option<f32> {
    if !segment_clear(a, b, obstacles) {
        return None;
    }
    let mut penalty = 0.0;
    if parallel_conflict_indexed(a, b, index, edge_start, edge_end) {
        match avoidance {
            EdgeAvoidance::Hard => return None,
            EdgeAvoidance::Soft => penalty += 600.0,
            EdgeAvoidance::Off => {}
        }
    }
    // Crossings are legal but expensive, so the router only accepts one when
    // the alternative would be a much longer detour.
    if avoidance != EdgeAvoidance::Off {
        penalty += crossing_penalty_indexed(a, b, index, edge_start, edge_end);
    }
    Some(penalty)
}

fn relax(
    state: usize,
    cost: f32,
    previous_state: usize,
    dist: &mut [f32],
    previous: &mut [usize],
    heap: &mut BinaryHeap<Reverse<(i64, usize)>>,
) {
    if cost + 1e-3 < dist[state] {
        dist[state] = cost;
        previous[state] = previous_state;
        heap.push(Reverse(((cost * 10.0) as i64, state)));
    }
}

fn sorted_unique(mut values: Vec<f32>) -> Vec<f32> {
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    values.dedup_by(|a, b| (*a - *b).abs() < 0.5);
    values
}

fn index_of(values: &[f32], value: f32) -> Option<usize> {
    values.iter().position(|item| (*item - value).abs() < 0.01)
}

fn simplify_orthogonal(
    path: &mut Vec<[f32; 2]>,
    obstacles: &[Rect],
    segments: &SegmentIndex,
    edge_start: [f32; 2],
    edge_end: [f32; 2],
    avoidance: EdgeAvoidance,
) {
    let mut index = 0;
    while index + 2 < path.len() {
        let a = path[index];
        let b = path[index + 1];
        let c = path[index + 2];
        let collinear = ((a[0] - b[0]).abs() < 0.1 && (b[0] - c[0]).abs() < 0.1)
            || ((a[1] - b[1]).abs() < 0.1 && (b[1] - c[1]).abs() < 0.1);
        if collinear {
            path.remove(index + 1);
        } else {
            index += 1;
        }
    }

    let check = if avoidance == EdgeAvoidance::Soft {
        EdgeAvoidance::Hard
    } else {
        avoidance
    };
    let mut changed = true;
    while changed {
        changed = false;
        let mut index = 0;
        while index + 2 < path.len() {
            let a = path[index];
            let c = path[index + 2];
            let axis_aligned = (a[0] - c[0]).abs() < 0.1 || (a[1] - c[1]).abs() < 0.1;
            let crossing_ok = avoidance == EdgeAvoidance::Off
                || crossing_penalty_indexed(a, c, segments, edge_start, edge_end) == 0.0;
            if axis_aligned
                && crossing_ok
                && segment_ok_indexed(a, c, obstacles, segments, edge_start, edge_end, check)
            {
                path.remove(index + 1);
                changed = true;
            } else {
                index += 1;
            }
        }
    }
}

fn route_between(
    start: [f32; 2],
    end: [f32; 2],
    obstacles: &[Rect],
    existing: &[Vec<[f32; 2]>],
    edge_start: [f32; 2],
    edge_end: [f32; 2],
    avoidance: EdgeAvoidance,
) -> Option<Vec<[f32; 2]>> {
    let mut path = if avoidance != EdgeAvoidance::Soft
        && segment_ok(
            start, end, obstacles, existing, edge_start, edge_end, avoidance,
        ) {
        vec![start, end]
    } else {
        visibility_route(
            start, end, obstacles, existing, edge_start, edge_end, avoidance,
        )?
    };
    simplify_path(
        &mut path, obstacles, existing, edge_start, edge_end, avoidance,
    );
    Some(path)
}

fn segment_ok(
    a: [f32; 2],
    b: [f32; 2],
    obstacles: &[Rect],
    existing: &[Vec<[f32; 2]>],
    edge_start: [f32; 2],
    edge_end: [f32; 2],
    avoidance: EdgeAvoidance,
) -> bool {
    if !segment_clear(a, b, obstacles) {
        return false;
    }
    if avoidance == EdgeAvoidance::Hard && parallel_conflict(a, b, existing, edge_start, edge_end) {
        return false;
    }
    true
}

fn segment_clear(a: [f32; 2], b: [f32; 2], obstacles: &[Rect]) -> bool {
    !obstacles
        .iter()
        .any(|rect| segment_hits_rect(a, b, &rect.expand(-0.5)))
}

fn segment_hits_rect(a: [f32; 2], b: [f32; 2], rect: &Rect) -> bool {
    let dx = b[0] - a[0];
    let dy = b[1] - a[1];
    let mut t0 = 0.0f32;
    let mut t1 = 1.0f32;
    let p = [-dx, dx, -dy, dy];
    let q = [
        a[0] - rect.x,
        rect.right() - a[0],
        a[1] - rect.y,
        rect.bottom() - a[1],
    ];
    for index in 0..4 {
        if p[index].abs() < 1e-6 {
            if q[index] < 0.0 {
                return false;
            }
        } else {
            let t = q[index] / p[index];
            if p[index] < 0.0 {
                t0 = t0.max(t);
            } else {
                t1 = t1.min(t);
            }
            if t0 > t1 {
                return false;
            }
        }
    }
    t1 > 0.02 && t0 < 0.98
}

fn inside_rect_strict(point: [f32; 2], rect: &Rect) -> bool {
    point[0] > rect.x + 0.5
        && point[0] < rect.right() - 0.5
        && point[1] > rect.y + 0.5
        && point[1] < rect.bottom() - 0.5
}

fn visibility_route(
    start: [f32; 2],
    end: [f32; 2],
    obstacles: &[Rect],
    existing: &[Vec<[f32; 2]>],
    edge_start: [f32; 2],
    edge_end: [f32; 2],
    avoidance: EdgeAvoidance,
) -> Option<Vec<[f32; 2]>> {
    let mut vertices = vec![start, end];
    for rect in obstacles {
        for corner in [
            [rect.x, rect.y],
            [rect.right(), rect.y],
            [rect.x, rect.bottom()],
            [rect.right(), rect.bottom()],
        ] {
            if !obstacles
                .iter()
                .any(|other| inside_rect_strict(corner, other))
            {
                vertices.push(corner);
            }
        }
    }
    if avoidance != EdgeAvoidance::Off {
        add_offset_vertices(&mut vertices, existing, obstacles);
    }
    dedup_vertices(&mut vertices);

    let start_index = vertices
        .iter()
        .position(|point| same_point(*point, start))?;
    let end_index = vertices.iter().position(|point| same_point(*point, end))?;
    let count = vertices.len();
    let mut dist = vec![f32::INFINITY; count];
    let mut previous: Vec<Option<usize>> = vec![None; count];
    let mut visited = vec![false; count];
    dist[start_index] = 0.0;

    for _ in 0..count {
        let mut current = None;
        let mut best = f32::INFINITY;
        for index in 0..count {
            if !visited[index] && dist[index] < best {
                best = dist[index];
                current = Some(index);
            }
        }
        let Some(current) = current else {
            break;
        };
        if current == end_index {
            break;
        }
        visited[current] = true;
        for next in 0..count {
            if visited[next] {
                continue;
            }
            let a = vertices[current];
            let b = vertices[next];
            if !segment_ok(a, b, obstacles, existing, edge_start, edge_end, avoidance) {
                continue;
            }
            let length = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
            let cost = dist[current] + length + 1.5;
            if cost < dist[next] {
                dist[next] = cost;
                previous[next] = Some(current);
            }
        }
    }

    if !dist[end_index].is_finite() {
        return None;
    }
    let mut path = Vec::new();
    let mut current = Some(end_index);
    while let Some(index) = current {
        path.push(vertices[index]);
        if index == start_index {
            break;
        }
        current = previous[index];
    }
    path.reverse();
    Some(path)
}

fn add_offset_vertices(
    vertices: &mut Vec<[f32; 2]>,
    existing: &[Vec<[f32; 2]>],
    obstacles: &[Rect],
) {
    for route in existing {
        for window in route.windows(2) {
            let a = window[0];
            let b = window[1];
            let length = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
            if length < 30.0 {
                continue;
            }
            let u = [(b[0] - a[0]) / length, (b[1] - a[1]) / length];
            let n = [u[1], -u[0]];
            for t in [0.1, 0.25, 0.5, 0.75, 0.9] {
                let p = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
                for sign in [-1.0, 1.0] {
                    let offset = (EDGE_CLEARANCE + 2.0) * sign;
                    let point = [p[0] + n[0] * offset, p[1] + n[1] * offset];
                    if !obstacles.iter().any(|rect| inside_rect_strict(point, rect)) {
                        vertices.push(point);
                    }
                }
            }
        }
    }
}

fn parallel_conflict(
    a: [f32; 2],
    b: [f32; 2],
    existing: &[Vec<[f32; 2]>],
    edge_start: [f32; 2],
    edge_end: [f32; 2],
) -> bool {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let ab_length = (ab[0] * ab[0] + ab[1] * ab[1]).sqrt();
    if ab_length < 1e-3 {
        return false;
    }
    for route in existing {
        for window in route.windows(2) {
            let Some((c, d)) = trim_segment(window[0], window[1], edge_start, SHARED_RADIUS) else {
                continue;
            };
            let Some((c, d)) = trim_segment(c, d, edge_end, SHARED_RADIUS) else {
                continue;
            };
            let cd = [d[0] - c[0], d[1] - c[1]];
            let cd_length = (cd[0] * cd[0] + cd[1] * cd[1]).sqrt();
            if cd_length < 1e-3 {
                continue;
            }
            let sin = (ab[0] * cd[1] - ab[1] * cd[0]).abs() / (ab_length * cd_length);
            if sin >= 0.25 {
                continue;
            }
            if segment_distance(a, b, c, d) < EDGE_CLEARANCE {
                return true;
            }
        }
    }
    false
}

fn trim_segment(
    mut c: [f32; 2],
    mut d: [f32; 2],
    point: [f32; 2],
    radius: f32,
) -> Option<([f32; 2], [f32; 2])> {
    let length = ((d[0] - c[0]).powi(2) + (d[1] - c[1]).powi(2)).sqrt();
    if length < 1.0 {
        return None;
    }
    let u = [(d[0] - c[0]) / length, (d[1] - c[1]) / length];
    let trim = radius.min(length * 0.45);
    if point_segment_distance(point, c, d) < radius {
        if distance_to_point(c, point) < radius {
            c = [c[0] + u[0] * trim, c[1] + u[1] * trim];
        } else {
            d = [d[0] - u[0] * trim, d[1] - u[1] * trim];
        }
    }
    if distance_to_point(c, d) < 1.0 {
        None
    } else {
        Some((c, d))
    }
}

fn segment_distance(a: [f32; 2], b: [f32; 2], c: [f32; 2], d: [f32; 2]) -> f32 {
    point_segment_distance(a, c, d)
        .min(point_segment_distance(b, c, d))
        .min(point_segment_distance(c, a, b))
        .min(point_segment_distance(d, a, b))
}

fn point_segment_distance(point: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let length2 = ab[0] * ab[0] + ab[1] * ab[1];
    if length2 < 1e-6 {
        return distance_to_point(point, a);
    }
    let t = (((point[0] - a[0]) * ab[0] + (point[1] - a[1]) * ab[1]) / length2).clamp(0.0, 1.0);
    let q = [a[0] + ab[0] * t, a[1] + ab[1] * t];
    distance_to_point(point, q)
}

fn distance_to_point(a: [f32; 2], b: [f32; 2]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

fn simplify_path(
    path: &mut Vec<[f32; 2]>,
    obstacles: &[Rect],
    existing: &[Vec<[f32; 2]>],
    edge_start: [f32; 2],
    edge_end: [f32; 2],
    avoidance: EdgeAvoidance,
) {
    let mut index = 0;
    while index + 2 < path.len() {
        if segment_ok(
            path[index],
            path[index + 2],
            obstacles,
            existing,
            edge_start,
            edge_end,
            if avoidance == EdgeAvoidance::Soft {
                EdgeAvoidance::Hard
            } else {
                avoidance
            },
        ) {
            path.remove(index + 1);
        } else {
            index += 1;
        }
    }
}

fn route_label_pos(points: &[[f32; 2]]) -> [f32; 2] {
    let mut best_length = 0.0f32;
    let mut midpoint = [0.0, 0.0];
    let mut horizontal = true;
    for window in points.windows(2) {
        let dx = window[1][0] - window[0][0];
        let dy = window[1][1] - window[0][1];
        let length = dx * dx + dy * dy;
        if length > best_length {
            best_length = length;
            midpoint = [
                (window[0][0] + window[1][0]) / 2.0,
                (window[0][1] + window[1][1]) / 2.0,
            ];
            horizontal = dx.abs() >= dy.abs();
        }
    }
    if horizontal {
        [midpoint[0], midpoint[1] - 12.0]
    } else {
        [midpoint[0] + 10.0, midpoint[1]]
    }
}

fn same_point(a: [f32; 2], b: [f32; 2]) -> bool {
    (a[0] - b[0]).abs() < 0.25 && (a[1] - b[1]).abs() < 0.25
}

fn dedup_points(points: &mut Vec<[f32; 2]>) {
    points.dedup_by(|a, b| same_point(*a, *b));
}

fn dedup_vertices(vertices: &mut Vec<[f32; 2]>) {
    let mut index = 0;
    while index < vertices.len() {
        let mut next = index + 1;
        while next < vertices.len() {
            if same_point(vertices[index], vertices[next]) {
                vertices.remove(next);
            } else {
                next += 1;
            }
        }
        index += 1;
    }
}

fn compute_bounds(doc: &Document, layout: &NetworkLayout) -> Rect {
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for node in &doc.nodes {
        if let Some(rect) = layout.node_rects.get(&node.id) {
            min_x = min_x.min(rect.x - 40.0);
            min_y = min_y.min(rect.y - 44.0);
            max_x = max_x.max(rect.right() + 40.0);
            max_y = max_y.max(rect.bottom() + 40.0);
        }
    }
    for route in layout.edge_routes.values() {
        for p in &route.points {
            min_x = min_x.min(p[0] - 20.0);
            min_y = min_y.min(p[1] - 12.0);
            max_x = max_x.max(p[0] + 20.0);
            max_y = max_y.max(p[1] + 12.0);
        }
    }
    if min_x.is_finite() {
        Rect::new(min_x, min_y, max_x - min_x, max_y - min_y)
    } else {
        Rect::new(0.0, 0.0, 800.0, 500.0)
    }
}

pub fn input_port_pos(node: &crate::model::Node, rect: Rect, port: usize) -> [f32; 2] {
    if matches!(node.kind, crate::model::NodeKind::Snapshot { .. }) && port == 1 {
        return [rect.x + rect.w / 2.0, rect.bottom()];
    }
    if matches!(node.kind, crate::model::NodeKind::Hold { .. }) && port == 0 {
        return [rect.x + rect.w / 2.0, rect.y];
    }
    let count = node.kind.input_types().len();
    let t = if count <= 1 || matches!(node.kind, crate::model::NodeKind::Snapshot { .. }) {
        0.5
    } else if node.kind.is_component() {
        0.22 + 0.56 * port as f32 / (count - 1) as f32
    } else {
        0.3 + 0.4 * port as f32 / (count - 1) as f32
    };
    [rect.x, rect.y + rect.h * t]
}

pub fn output_port_pos(node: &crate::model::Node, rect: Rect, port: usize) -> [f32; 2] {
    let count = node.kind.output_types().len();
    let t = if count <= 1 {
        0.5
    } else {
        0.25 + 0.5 * port as f32 / (count - 1) as f32
    };
    [rect.right(), rect.y + rect.h * t]
}

pub fn auto_layout_tree(doc: &mut Document) {
    auto_layout(doc);
    for definition in &mut doc.components {
        auto_layout_tree(&mut definition.doc);
    }
}

pub fn auto_layout(doc: &mut Document) {
    if doc.nodes.is_empty() {
        return;
    }
    let ids: Vec<NodeId> = doc.nodes.iter().map(|n| n.id).collect();
    let index: HashMap<NodeId, usize> = ids.iter().enumerate().map(|(i, id)| (*id, i)).collect();
    let sizes: HashMap<NodeId, [f32; 2]> = doc
        .nodes
        .iter()
        .map(|node| (node.id, node.size()))
        .collect();
    let mut neighbors: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    for edge in &doc.edges {
        if !index.contains_key(&edge.from) || !index.contains_key(&edge.to) {
            continue;
        }
        neighbors.entry(edge.from).or_default().push(edge.to);
        neighbors.entry(edge.to).or_default().push(edge.from);
    }
    let mut incoming: HashMap<NodeId, Vec<(NodeId, usize)>> = HashMap::new();
    for edge in &doc.edges {
        if !index.contains_key(&edge.from) || !index.contains_key(&edge.to) {
            continue;
        }
        incoming
            .entry(edge.to)
            .or_default()
            .push((edge.from, edge.from_port));
    }
    let mut adjacency: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    for edge in &doc.edges {
        if !index.contains_key(&edge.from) || !index.contains_key(&edge.to) {
            continue;
        }
        let feedback_to_snapshot = doc
            .node(edge.to)
            .map(|node| {
                matches!(node.kind, crate::model::NodeKind::Snapshot { .. })
                    && edge.to_port == 1
                    && doc
                        .node(edge.from)
                        .map(|source| matches!(source.kind, crate::model::NodeKind::Hold { .. }))
                        .unwrap_or(false)
            })
            .unwrap_or(false);
        if feedback_to_snapshot {
            continue;
        }
        adjacency.entry(edge.from).or_default().push(edge.to);
    }

    let mut color: HashMap<NodeId, u8> = ids.iter().map(|id| (*id, 0)).collect();
    let mut back_edges: HashSet<(NodeId, NodeId)> = HashSet::new();
    for &id in &ids {
        if color.get(&id).copied().unwrap_or(0) == 0 {
            dfs_mark(id, &adjacency, &mut color, &mut back_edges);
        }
    }

    let mut indegree: HashMap<NodeId, usize> = ids.iter().map(|id| (*id, 0)).collect();
    let mut dag: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    for (&from, targets) in &adjacency {
        for &to in targets {
            if back_edges.contains(&(from, to)) {
                continue;
            }
            dag.entry(from).or_default().push(to);
            *indegree.entry(to).or_default() += 1;
        }
    }
    let mut queue: Vec<NodeId> = ids
        .iter()
        .copied()
        .filter(|id| indegree.get(id).copied().unwrap_or(0) == 0)
        .collect();
    queue.sort_by_key(|id| index.get(id).copied().unwrap_or(0));
    let mut layer: HashMap<NodeId, usize> = ids.iter().map(|id| (*id, 0)).collect();
    let mut qidx = 0;
    while qidx < queue.len() {
        let id = queue[qidx];
        qidx += 1;
        let current = layer.get(&id).copied().unwrap_or(0);
        if let Some(next) = dag.get(&id) {
            let mut ready = Vec::new();
            for &to in next {
                let entry = layer.entry(to).or_insert(0);
                *entry = (*entry).max(current + 1);
                if let Some(deg) = indegree.get_mut(&to) {
                    *deg = deg.saturating_sub(1);
                    if *deg == 0 {
                        ready.push(to);
                    }
                }
            }
            ready.sort_by_key(|id| index.get(id).copied().unwrap_or(0));
            queue.extend(ready);
        }
    }

    let max_layer = layer.values().copied().max().unwrap_or(0);
    let mut groups: Vec<Vec<NodeId>> = vec![Vec::new(); max_layer + 1];
    for id in &ids {
        groups[layer.get(id).copied().unwrap_or(0)].push(*id);
    }

    // Sinks are terminals. Put them in their own rightmost column so an
    // incoming wire never has to run backwards around unrelated nodes.
    let sink_ids: HashSet<NodeId> = doc
        .nodes
        .iter()
        .filter(|node| node.kind.is_sink())
        .map(|node| node.id)
        .collect();
    if !sink_ids.is_empty() {
        for group in &mut groups {
            group.retain(|id| !sink_ids.contains(id));
        }
        groups.retain(|group| !group.is_empty());
        let mut sinks: Vec<NodeId> = sink_ids.iter().copied().collect();
        sinks.sort_by_key(|id| index.get(id).copied().unwrap_or(0));
        groups.push(sinks);
    }

    let mut y_order: HashMap<NodeId, f32> = doc.nodes.iter().map(|n| (n.id, n.pos[1])).collect();
    for _ in 0..3 {
        for group in groups.iter_mut() {
            group.sort_by(|a, b| {
                let ay = neighbor_center(*a, &neighbors, &y_order);
                let by = neighbor_center(*b, &neighbors, &y_order);
                ay.partial_cmp(&by)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| {
                        index
                            .get(a)
                            .copied()
                            .unwrap_or(0)
                            .cmp(&index.get(b).copied().unwrap_or(0))
                    })
            });
        }
        for (li, group) in groups.iter().enumerate() {
            for (row, id) in group.iter().enumerate() {
                y_order.insert(*id, (li * 1000 + row) as f32);
            }
        }
    }

    // Align each node with its incoming port anchors instead of centering
    // every layer in the tallest column. This keeps straight chains straight;
    // sinks in particular follow the source output port that feeds them.
    let row_gap = 42.0;
    let sink_row_gap = 24.0;
    let group_gap = |group: &[NodeId]| {
        if group.iter().all(|id| sink_ids.contains(id)) {
            sink_row_gap
        } else {
            row_gap
        }
    };
    let mut centers: HashMap<NodeId, f32> = HashMap::new();
    for group in &groups {
        let gap = group_gap(group);
        let mut y = 80.0;
        for id in group {
            let height = node_height(&sizes, *id);
            centers.insert(*id, y + height / 2.0);
            y += height + gap;
        }
    }
    for _ in 0..20 {
        for group in &mut groups {
            let mut desired: Vec<(NodeId, f32)> = group
                .iter()
                .map(|id| {
                    (
                        *id,
                        desired_center_y(*id, doc, &sizes, &centers, &incoming, &neighbors),
                    )
                })
                .collect();
            desired.sort_by(|(a_id, a), (b_id, b)| {
                a.partial_cmp(b)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| {
                        index
                            .get(a_id)
                            .copied()
                            .unwrap_or(0)
                            .cmp(&index.get(b_id).copied().unwrap_or(0))
                    })
            });
            *group = desired.iter().map(|(id, _)| *id).collect();
            let mut centers_in_group: Vec<f32> =
                desired.into_iter().map(|(_, center)| center).collect();
            enforce_spacing(&sizes, group, &mut centers_in_group, group_gap(group));
            for (id, center) in group.iter().zip(centers_in_group) {
                centers.insert(*id, center);
            }
        }
    }

    let min_top = doc
        .nodes
        .iter()
        .filter_map(|node| {
            centers.get(&node.id).map(|center| {
                center - sizes.get(&node.id).map(|size| size[1]).unwrap_or(74.0) / 2.0
            })
        })
        .fold(f32::INFINITY, f32::min);
    let y_shift = if min_top.is_finite() {
        80.0 - min_top
    } else {
        0.0
    };

    let column_gap = 160.0;
    let mut layer_x = Vec::with_capacity(groups.len());
    let mut x = 80.0;
    for group in &groups {
        layer_x.push(x);
        let width = group
            .iter()
            .filter_map(|id| sizes.get(id).map(|size| size[0]))
            .fold(0.0f32, f32::max);
        x += width + column_gap;
    }
    for (li, group) in groups.iter().enumerate() {
        for id in group {
            let Some(node) = doc.node_mut(*id) else {
                continue;
            };
            let height = sizes.get(id).map(|size| size[1]).unwrap_or(74.0);
            let center = centers.get(id).copied().unwrap_or(80.0) + y_shift;
            node.pos = [layer_x[li], center - height / 2.0];
        }
    }
}

/// Barycenter in the coordinate system of incoming ports rather than node
/// centers. Nodes without incoming edges fall back to neighbour centering.
fn desired_center_y(
    id: NodeId,
    doc: &Document,
    sizes: &HashMap<NodeId, [f32; 2]>,
    centers: &HashMap<NodeId, f32>,
    incoming: &HashMap<NodeId, Vec<(NodeId, usize)>>,
    neighbors: &HashMap<NodeId, Vec<NodeId>>,
) -> f32 {
    if let Some(sources) = incoming.get(&id) {
        let mut sum = 0.0;
        let mut count = 0.0;
        for (source, port) in sources {
            if let Some(y) = source_port_y(*source, *port, doc, sizes, centers) {
                sum += y;
                count += 1.0;
            }
        }
        if count > 0.0 {
            return sum / count;
        }
    }
    neighbor_center(id, neighbors, centers)
}

fn source_port_y(
    id: NodeId,
    port: usize,
    doc: &Document,
    sizes: &HashMap<NodeId, [f32; 2]>,
    centers: &HashMap<NodeId, f32>,
) -> Option<f32> {
    let size = sizes.get(&id)?;
    let center = centers.get(&id)?;
    let count = doc.node(id)?.kind.output_types().len();
    if count == 0 {
        return None;
    }
    let t = if count <= 1 {
        0.5
    } else {
        0.25 + 0.5 * port.min(count - 1) as f32 / (count - 1) as f32
    };
    Some(center - size[1] / 2.0 + size[1] * t)
}

fn node_height(sizes: &HashMap<NodeId, [f32; 2]>, id: NodeId) -> f32 {
    sizes.get(&id).map(|size| size[1]).unwrap_or(74.0)
}

fn enforce_spacing(
    sizes: &HashMap<NodeId, [f32; 2]>,
    group: &[NodeId],
    centers: &mut [f32],
    gap: f32,
) {
    for index in 1..group.len() {
        let min_gap =
            (node_height(sizes, group[index - 1]) + node_height(sizes, group[index])) / 2.0 + gap;
        centers[index] = centers[index].max(centers[index - 1] + min_gap);
    }
    for index in (0..group.len().saturating_sub(1)).rev() {
        let min_gap =
            (node_height(sizes, group[index]) + node_height(sizes, group[index + 1])) / 2.0 + gap;
        centers[index] = centers[index].min(centers[index + 1] - min_gap);
    }
    for index in 1..group.len() {
        let min_gap =
            (node_height(sizes, group[index - 1]) + node_height(sizes, group[index])) / 2.0 + gap;
        centers[index] = centers[index].max(centers[index - 1] + min_gap);
    }
}

fn neighbor_center(
    id: NodeId,
    neighbors: &HashMap<NodeId, Vec<NodeId>>,
    y_order: &HashMap<NodeId, f32>,
) -> f32 {
    let fallback = y_order.get(&id).copied().unwrap_or(0.0);
    let Some(near) = neighbors.get(&id) else {
        return fallback;
    };
    let mut sum = 0.0;
    let mut count = 0.0;
    for other in near {
        if let Some(y) = y_order.get(other) {
            sum += *y;
            count += 1.0;
        }
    }
    if count == 0.0 {
        fallback
    } else {
        sum / count
    }
}

fn dfs_mark(
    id: NodeId,
    adjacency: &HashMap<NodeId, Vec<NodeId>>,
    color: &mut HashMap<NodeId, u8>,
    back_edges: &mut HashSet<(NodeId, NodeId)>,
) {
    color.insert(id, 1);
    if let Some(next) = adjacency.get(&id) {
        for &to in next {
            match color.get(&to).copied().unwrap_or(0) {
                0 => dfs_mark(to, adjacency, color, back_edges),
                1 => {
                    back_edges.insert((id, to));
                }
                _ => {}
            }
        }
    }
    color.insert(id, 2);
}

const HOP_RADIUS: f32 = 5.0;

fn add_crossing_hops(routes: &mut HashMap<u64, EdgeRoute>) {
    let mut ids: Vec<u64> = routes.keys().copied().collect();
    ids.sort_unstable();
    let mut hops: HashMap<u64, Vec<[f32; 2]>> = HashMap::new();

    for i in 0..ids.len() {
        for j in (i + 1)..ids.len() {
            let a_id = ids[i];
            let b_id = ids[j];
            let a = &routes[&a_id];
            let b = &routes[&b_id];
            for wa in a.points.windows(2) {
                for wb in b.points.windows(2) {
                    let Some(point) = segment_crossing(wa[0], wa[1], wb[0], wb[1]) else {
                        continue;
                    };
                    let target = a_id.max(b_id);
                    let list = hops.entry(target).or_default();
                    if !list.iter().any(|p| dist2(*p, point) < 16.0) {
                        list.push(point);
                    }
                }
            }
        }
    }

    for (id, points) in hops {
        if let Some(route) = routes.get_mut(&id) {
            route.points = insert_hops(&route.points, &points, HOP_RADIUS);
        }
    }
}

fn segment_crossing(a: [f32; 2], b: [f32; 2], c: [f32; 2], d: [f32; 2]) -> Option<[f32; 2]> {
    let r = [b[0] - a[0], b[1] - a[1]];
    let s = [d[0] - c[0], d[1] - c[1]];
    let denom = cross(r, s);
    if denom.abs() < 1e-4 {
        return None;
    }
    let ca = [c[0] - a[0], c[1] - a[1]];
    let t = cross(ca, s) / denom;
    let u = cross(ca, r) / denom;
    if !(0.06..=0.94).contains(&t) || !(0.06..=0.94).contains(&u) {
        return None;
    }
    Some([a[0] + r[0] * t, a[1] + r[1] * t])
}

fn insert_hops(points: &[[f32; 2]], hops: &[[f32; 2]], radius: f32) -> Vec<[f32; 2]> {
    let mut result = points.to_vec();
    for hop in hops {
        let mut index = 0;
        while index + 1 < result.len() {
            let a = result[index];
            let b = result[index + 1];
            let Some(t) = point_on_segment(*hop, a, b) else {
                index += 1;
                continue;
            };
            let length = dist2(a, b).sqrt();
            if length <= radius * 3.0 || t * length <= radius || (1.0 - t) * length <= radius {
                index += 1;
                continue;
            }
            let u = [(b[0] - a[0]) / length, (b[1] - a[1]) / length];
            let n = [u[1], -u[0]];
            let steps = 8;
            let arc: Vec<[f32; 2]> = (0..=steps)
                .map(|step| {
                    let theta = std::f32::consts::PI * (1.0 - step as f32 / steps as f32);
                    [
                        hop[0] + u[0] * radius * theta.cos() + n[0] * radius * theta.sin(),
                        hop[1] + u[1] * radius * theta.cos() + n[1] * radius * theta.sin(),
                    ]
                })
                .collect();
            result.splice(index + 1..index + 1, arc);
            break;
        }
    }
    result
}

fn point_on_segment(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> Option<f32> {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let length2 = ab[0] * ab[0] + ab[1] * ab[1];
    if length2 < 1e-6 {
        return None;
    }
    let t = ((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / length2;
    if !(0.0..=1.0).contains(&t) {
        return None;
    }
    let q = [a[0] + ab[0] * t, a[1] + ab[1] * t];
    if dist2(q, p) <= 0.25 {
        Some(t)
    } else {
        None
    }
}

fn cross(a: [f32; 2], b: [f32; 2]) -> f32 {
    a[0] * b[1] - a[1] * b[0]
}

fn dist2(a: [f32; 2], b: [f32; 2]) -> f32 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)
}

pub fn edge_tip(route: &EdgeRoute) -> [f32; 2] {
    *route.points.last().unwrap_or(&[0.0, 0.0])
}

pub fn edge_direction(route: &EdgeRoute) -> [f32; 2] {
    if route.points.len() < 2 {
        return [1.0, 0.0];
    }
    let end = route.points[route.points.len() - 1];
    let prev = route.points[route.points.len() - 2];
    [end[0] - prev[0], end[1] - prev[1]]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::examples;
    use std::path::Path;

    fn trimmed_segments(points: &[[f32; 2]]) -> Vec<([f32; 2], [f32; 2])> {
        let (Some(start), Some(end)) = (points.first().copied(), points.last().copied()) else {
            return Vec::new();
        };
        let mut segments = Vec::new();
        for window in points.windows(2) {
            let Some((a, b)) = trim_segment(window[0], window[1], start, SHARED_RADIUS) else {
                continue;
            };
            let Some((a, b)) = trim_segment(a, b, end, SHARED_RADIUS) else {
                continue;
            };
            let length = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
            if length < 20.0 {
                continue;
            }
            segments.push((a, b));
        }
        segments
    }

    fn parallel_close(a: [f32; 2], b: [f32; 2], c: [f32; 2], d: [f32; 2]) -> bool {
        let ab = [b[0] - a[0], b[1] - a[1]];
        let cd = [d[0] - c[0], d[1] - c[1]];
        let ab_length = (ab[0] * ab[0] + ab[1] * ab[1]).sqrt();
        let cd_length = (cd[0] * cd[0] + cd[1] * cd[1]).sqrt();
        if ab_length < 1e-3 || cd_length < 1e-3 {
            return false;
        }
        let sin = (ab[0] * cd[1] - ab[1] * cd[0]).abs() / (ab_length * cd_length);
        sin < 0.25 && segment_distance(a, b, c, d) < EDGE_CLEARANCE - 0.5
    }

    fn fanout_fork_index(doc: &Document, edge: &Edge, route: &EdgeRoute) -> Option<usize> {
        let fanout_edges = doc
            .edges
            .iter()
            .filter(|other| other.from == edge.from && other.from_port == edge.from_port)
            .count();
        if fanout_edges < 3 {
            return None;
        }
        let fork = route.fork?;
        route
            .points
            .iter()
            .position(|point| same_point(*point, fork))
    }

    fn check_routes(key: &str, doc: &Document, network: &NetworkLayout) {
        let mut routes: Vec<_> = network.edge_routes.iter().collect();
        routes.sort_by_key(|(id, _)| **id);
        for i in 0..routes.len() {
            for j in i + 1..routes.len() {
                let a_edge = doc.edge(*routes[i].0);
                let b_edge = doc.edge(*routes[j].0);
                let same_fanout_group = match (a_edge, b_edge) {
                    (Some(a), Some(b)) => {
                        let count = doc
                            .edges
                            .iter()
                            .filter(|other| other.from == a.from && other.from_port == a.from_port)
                            .count();
                        a.from == b.from && a.from_port == b.from_port && count >= 3
                    }
                    _ => false,
                };
                let (a_start, b_start) = if same_fanout_group {
                    (
                        a_edge
                            .and_then(|edge| fanout_fork_index(doc, edge, routes[i].1))
                            .unwrap_or(0),
                        b_edge
                            .and_then(|edge| fanout_fork_index(doc, edge, routes[j].1))
                            .unwrap_or(0),
                    )
                } else {
                    (0, 0)
                };
                let a_segments = trimmed_segments(&routes[i].1.points[a_start..]);
                let b_segments = trimmed_segments(&routes[j].1.points[b_start..]);
                for &(a, b) in &a_segments {
                    for &(c, d) in &b_segments {
                        assert!(
                            !parallel_close(a, b, c, d),
                            "{key}: edges {} and {} run together\nsegments {:?}-{:?} and {:?}-{:?}\n{:?}\n{:?}",
                            routes[i].0,
                            routes[j].0,
                            a,
                            b,
                            c,
                            d,
                            routes[i].1.points,
                            routes[j].1.points
                        );
                    }
                }
            }
        }
    }

    fn crossing_count(network: &NetworkLayout) -> usize {
        let mut routes: Vec<_> = network.edge_routes.values().collect();
        routes.sort_by_key(|route| route.points.first().map(|point| point[0] as i64));
        let mut count = 0;
        for i in 0..routes.len() {
            for j in i + 1..routes.len() {
                for a in routes[i].points.windows(2) {
                    for b in routes[j].points.windows(2) {
                        if segment_crossing(a[0], a[1], b[0], b[1]).is_some() {
                            count += 1;
                        }
                    }
                }
            }
        }
        count
    }

    #[test]
    fn auto_layout_aligns_single_input_chains_and_sinks() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/modular/main.frp");
        let doc = crate::dsl::parse_file(&path).unwrap().doc;
        let network = layout(&doc);
        for (from, to) in [
            ("sDelta", "pipeline"),
            ("pipeline", "observed"),
            ("observed", "out"),
            ("pipeline", "state"),
        ] {
            let from_id = doc.nodes.iter().find(|node| node.name == from).unwrap().id;
            let to_id = doc.nodes.iter().find(|node| node.name == to).unwrap().id;
            let edge = doc
                .edges
                .iter()
                .find(|edge| edge.from == from_id && edge.to == to_id)
                .unwrap();
            let route = network.edge_routes.get(&edge.id).unwrap();
            assert!(
                route
                    .points
                    .windows(2)
                    .all(|pair| (pair[0][1] - pair[1][1]).abs() < 0.5),
                "{from} -> {to} should be horizontal: {:?}",
                route.points
            );
        }
        assert_eq!(crossing_count(&network), 0);
    }

    fn assert_routes_avoid_labels(doc: &Document, network: &NetworkLayout, key: &str) {
        for node in &doc.nodes {
            let Some(rect) = network.node_rects.get(&node.id).copied() else {
                continue;
            };
            let Some(label) = node_label_rect(node, rect) else {
                continue;
            };
            let label = label.expand(1.0);
            for (edge_id, route) in &network.edge_routes {
                for pair in route.points.windows(2) {
                    assert!(
                        !segment_hits_rect(pair[0], pair[1], &label),
                        "{key}: edge {edge_id} crosses label on node {}",
                        node.name
                    );
                }
            }
        }
    }

    #[test]
    fn auto_layout_keeps_routes_off_node_labels() {
        for (_, key) in examples::EXAMPLES {
            let mut doc = examples::load_example(key).unwrap();
            auto_layout(&mut doc);
            let network = layout(&doc);
            assert_routes_avoid_labels(&doc, &network, key);
        }
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/modular/main.frp");
        let doc = crate::dsl::parse_file(&path).unwrap().doc;
        let network = layout(&doc);
        assert_routes_avoid_labels(&doc, &network, "modular");
    }

    #[test]
    fn auto_example_routes_do_not_cross() {
        for (_, key) in examples::EXAMPLES {
            let mut doc = examples::load_example(key).unwrap();
            auto_layout(&mut doc);
            assert_eq!(
                crossing_count(&layout(&doc)),
                0,
                "{key} has crossing routes after auto layout"
            );
        }
    }

    #[test]
    fn component_ports_are_distributed() {
        use crate::model::{Node, NodeKind, PortSpec, PortType};
        let inputs = vec![
            PortSpec {
                name: "in".into(),
                ty: PortType::Stream,
            },
            PortSpec {
                name: "reset".into(),
                ty: PortType::Stream,
            },
        ];
        let outputs = vec![
            PortSpec {
                name: "a".into(),
                ty: PortType::Stream,
            },
            PortSpec {
                name: "b".into(),
                ty: PortType::Cell,
            },
        ];
        let node = Node::new(
            1,
            NodeKind::Component {
                definition: None,
                inputs,
                outputs,
                body: Vec::new(),
            },
            [0.0, 0.0],
        );
        let rect = Rect::new(0.0, 0.0, 220.0, 140.0);
        let input_a = input_port_pos(&node, rect, 0);
        let input_b = input_port_pos(&node, rect, 1);
        let output_a = output_port_pos(&node, rect, 0);
        let output_b = output_port_pos(&node, rect, 1);
        assert!(input_a[1] < input_b[1]);
        assert!(output_a[1] < output_b[1]);
        assert_eq!(input_a[0], rect.x);
        assert_eq!(output_a[0], rect.right());
    }

    #[test]
    fn sinks_stay_downstream_and_follow_their_input() {
        let mut lines = vec![
            "diagram Large".to_string(),
            "layout auto".to_string(),
            "time_end 2".to_string(),
            String::new(),
        ];
        for index in 0..20 {
            lines.push(format!("stream s{index} events 1:1"));
            lines.push(format!("map m{index} = identity"));
        }
        lines.push("map final = identity".into());
        lines.push("sink out".into());
        lines.push(String::new());
        for index in 0..20 {
            lines.push(format!("s{index} -> m{index}"));
        }
        lines.push("m0 -> final".into());
        lines.push("final -> out".into());
        let parsed = crate::dsl::parse(&lines.join("\n")).unwrap();
        let doc = parsed.doc;
        let source = doc.nodes.iter().find(|node| node.name == "m0").unwrap();
        let final_node = doc.nodes.iter().find(|node| node.name == "final").unwrap();
        let out = doc.nodes.iter().find(|node| node.name == "out").unwrap();
        assert!(final_node.pos[0] > source.pos[0]);
        assert!(out.pos[0] > final_node.pos[0]);
        let source_center = source.pos[1] + source.size()[1] / 2.0;
        let out_center = out.pos[1] + out.size()[1] / 2.0;
        assert!(
            (source_center - out_center).abs() < 60.0,
            "source_center={source_center}, out_center={out_center}"
        );
        let network = layout(&doc);
        for (from, to) in [(source.id, final_node.id), (final_node.id, out.id)] {
            let edge = doc
                .edges
                .iter()
                .find(|edge| edge.from == from && edge.to == to)
                .unwrap();
            let route = network.edge_routes.get(&edge.id).unwrap();
            for pair in route.points.windows(2) {
                assert!(
                    pair[1][0] + 0.5 >= pair[0][0],
                    "wire goes backwards: {:?}",
                    route.points
                );
            }
        }
    }

    #[test]
    fn fanout_wires_share_a_trunk_instead_of_weaving() {
        let source = "\
diagram Fan out
layout auto
time_end 2

stream s events 1:1
map m = identity
sink a
sink b
sink c

s -> m
m -> a
m -> b
m -> c
";
        let doc = crate::dsl::parse(source).unwrap().doc;
        let map = doc.nodes.iter().find(|node| node.name == "m").unwrap();
        let network = layout(&doc);
        let map_rect = network.node_rects[&map.id];
        let port = output_port_pos(map, map_rect, 0);

        let mut routes = Vec::new();
        for name in ["a", "b", "c"] {
            let sink = doc.nodes.iter().find(|node| node.name == name).unwrap();
            let edge = doc
                .edges
                .iter()
                .find(|edge| edge.from == map.id && edge.to == sink.id)
                .unwrap();
            let route = network.edge_routes.get(&edge.id).unwrap();
            assert_eq!(route.points.first().copied(), Some(port));
            assert!(
                route.points.len() <= 6,
                "{name} route should not weave: {:?}",
                route.points
            );
            routes.push(route);
        }

        let fork_x = routes[0].points[1][0];
        for route in &routes {
            assert!(
                (route.points[1][0] - fork_x).abs() < 0.5,
                "fan-out routes should share one trunk: {:?}",
                route.points
            );
        }
        assert!(port[0] < fork_x);
        assert_eq!(
            routes
                .iter()
                .filter(|route| route.junction.is_some())
                .count(),
            1,
            "the middle branch should mark its junction"
        );
    }

    #[test]
    fn routes_keep_clear_of_component_cloud_lobes() {
        let source = "\
diagram Cloud occlusion
layout manual
time_end 2

component wide {
  in a: stream
  in b: stream
  in c: stream
  in d: stream
  out x: stream
  out y: stream
  out z: stream
  out w: stream
  body \"cloud body line\"
}

stream s events 1:1 at 40,240
component cloud = wide at 300,160
sink out at 640,240

s -> out
";
        let doc = crate::dsl::parse(source).unwrap().doc;
        let network = layout(&doc);
        let cloud = doc.nodes.iter().find(|node| node.name == "cloud").unwrap();
        let rect = network.node_rects[&cloud.id];
        let points = cloud_points(rect);
        let (mut min_x, mut min_y) = (f32::INFINITY, f32::INFINITY);
        let (mut max_x, mut max_y) = (f32::NEG_INFINITY, f32::NEG_INFINITY);
        for point in &points {
            min_x = min_x.min(point[0]);
            min_y = min_y.min(point[1]);
            max_x = max_x.max(point[0]);
            max_y = max_y.max(point[1]);
        }
        let cloud_bounds = Rect::new(min_x, min_y, max_x - min_x, max_y - min_y);

        let source = doc.nodes.iter().find(|node| node.name == "s").unwrap();
        let sink = doc.nodes.iter().find(|node| node.name == "out").unwrap();
        let edge = doc
            .edges
            .iter()
            .find(|edge| edge.from == source.id && edge.to == sink.id)
            .unwrap();
        let route = network.edge_routes.get(&edge.id).unwrap();
        for pair in route.points.windows(2) {
            assert!(
                !segment_hits_rect(pair[0], pair[1], &cloud_bounds),
                "wire {}-{} runs under the component cloud: {:?}",
                source.name,
                sink.name,
                route.points
            );
        }
    }

    #[test]
    fn cloud_clearance_keeps_tight_corridors_routable() {
        use crate::model::{Node, PortSpec, PortType};
        let mut doc = Document::blank();
        let mut source = Node::new(
            0,
            NodeKind::InputEvent { events: Vec::new() },
            [61.16, 66.06],
        );
        source.name = "s".into();
        doc.nodes.push(source);
        let inputs = (0..4)
            .map(|index| PortSpec {
                name: format!("in{index}"),
                ty: PortType::Stream,
            })
            .collect();
        let outputs = (0..4)
            .map(|index| PortSpec {
                name: format!("out{index}"),
                ty: PortType::Stream,
            })
            .collect();
        let mut component = Node::new(
            0,
            NodeKind::Component {
                definition: None,
                inputs,
                outputs,
                body: vec!["cloud body line".into()],
            },
            [445.25, 185.13],
        );
        component.name = "cloud".into();
        doc.nodes.push(component);
        let mut sink = Node::new(0, NodeKind::Sink, [695.97, 308.49]);
        sink.name = "out".into();
        doc.nodes.push(sink);
        // Re-number the nodes so the layout sees a normal document.
        for (index, node) in doc.nodes.iter_mut().enumerate() {
            node.id = index as u64 + 1;
        }
        let from = doc.nodes[0].id;
        let to = doc.nodes[2].id;
        doc.add_edge(from, to, 0).unwrap();

        let network = layout(&doc);
        let route = network.edge_routes.get(&doc.edges[0].id).unwrap();
        for pair in route.points.windows(2) {
            assert!(
                (pair[0][0] - pair[1][0]).abs() < 0.5 || (pair[0][1] - pair[1][1]).abs() < 0.5,
                "tight cloud corridor should stay orthogonal: {:?}",
                route.points
            );
        }
    }

    #[test]
    fn fanout_trunk_routes_around_a_blocking_node() {
        let source = "\
diagram Fan out obstacle
layout manual
time_end 2

stream s events 1:1 at 40,200
map blocker = identity at 90,180
sink a at 500,100
sink b at 500,250
sink c at 500,400

s -> a
s -> b
s -> c
";
        let doc = crate::dsl::parse(source).unwrap().doc;
        let network = layout(&doc);
        let blocker = doc
            .nodes
            .iter()
            .find(|node| node.name == "blocker")
            .unwrap();
        let blocker_rect = network.node_rects[&blocker.id];
        for edge in &doc.edges {
            let route = network.edge_routes.get(&edge.id).unwrap();
            for pair in route.points.windows(2) {
                assert!(
                    (pair[0][0] - pair[1][0]).abs() < 0.5 || (pair[0][1] - pair[1][1]).abs() < 0.5,
                    "fan-out detour should stay orthogonal: {:?}",
                    route.points
                );
                assert!(
                    !segment_hits_rect(pair[0], pair[1], &blocker_rect),
                    "fan-out wire crosses an unrelated node: {:?}",
                    route.points
                );
            }
        }
    }

    #[test]
    fn distant_nodes_do_not_shrink_cloud_clearance() {
        let source = "\
diagram Cloud clearance
layout manual
time_end 2

component wide {
  in a: stream
  in b: stream
  in c: stream
  in d: stream
  out x: stream
  out y: stream
  out z: stream
  out w: stream
  body \"cloud body line\"
}

stream s events 1:1 at 40,314
component cloud = wide at 300,160
sink out at 640,314
sink far at 1000,330

s -> out
";
        let doc = crate::dsl::parse(source).unwrap().doc;
        let network = layout(&doc);
        let cloud = doc.nodes.iter().find(|node| node.name == "cloud").unwrap();
        let rect = network.node_rects[&cloud.id];
        let points = cloud_points(rect);
        let (mut min_x, mut min_y) = (f32::INFINITY, f32::INFINITY);
        let (mut max_x, mut max_y) = (f32::NEG_INFINITY, f32::NEG_INFINITY);
        for point in &points {
            min_x = min_x.min(point[0]);
            min_y = min_y.min(point[1]);
            max_x = max_x.max(point[0]);
            max_y = max_y.max(point[1]);
        }
        let cloud_bounds = Rect::new(min_x, min_y, max_x - min_x, max_y - min_y);

        let source = doc.nodes.iter().find(|node| node.name == "s").unwrap();
        let sink = doc.nodes.iter().find(|node| node.name == "out").unwrap();
        let edge = doc
            .edges
            .iter()
            .find(|edge| edge.from == source.id && edge.to == sink.id)
            .unwrap();
        let route = network.edge_routes.get(&edge.id).unwrap();
        for pair in route.points.windows(2) {
            assert!(
                !segment_hits_rect(pair[0], pair[1], &cloud_bounds),
                "a distant node shrank the cloud clearance: {:?}",
                route.points
            );
        }
    }

    #[test]
    fn fanout_branch_stubs_do_not_enter_a_blocker() {
        let source = "\
diagram Fan out blocker at fork
layout manual
time_end 2

stream s events 1:1 at 40,200
map blocker = identity at 130,180
sink a at 500,100
sink b at 500,200
sink c at 500,400

s -> a
s -> b
s -> c
";
        let doc = crate::dsl::parse(source).unwrap().doc;
        let network = layout(&doc);
        let blocker = doc
            .nodes
            .iter()
            .find(|node| node.name == "blocker")
            .unwrap();
        let blocker_rect = network.node_rects[&blocker.id];
        for edge in &doc.edges {
            let route = network.edge_routes.get(&edge.id).unwrap();
            for pair in route.points.windows(2) {
                assert!(
                    (pair[0][0] - pair[1][0]).abs() < 0.5 || (pair[0][1] - pair[1][1]).abs() < 0.5,
                    "fan-out should stay orthogonal: {:?}",
                    route.points
                );
                assert!(
                    !segment_hits_rect(pair[0], pair[1], &blocker_rect),
                    "fan-out branch start enters an unrelated node: {:?}",
                    route.points
                );
            }
        }
    }

    #[test]
    fn segment_index_matches_full_scan_conflicts() {
        let routes = vec![
            vec![[0.0, 0.0], [200.0, 0.0], [200.0, 120.0], [400.0, 120.0]],
            vec![[50.0, 300.0], [350.0, 300.0]],
            vec![[300.0, 0.0], [300.0, 250.0]],
            vec![[100.0, 50.0], [100.0, 250.0]],
            vec![[180.0, 220.0], [420.0, 220.0]],
        ];
        let index = SegmentIndex::new(&routes);
        let edge_start = [0.0, 0.0];
        let edge_end = [400.0, 120.0];
        let scan_crossing = |a: [f32; 2], b: [f32; 2]| {
            let mut penalty = 0.0;
            for route in &routes {
                for window in route.windows(2) {
                    let Some((c, d)) =
                        trim_segment(window[0], window[1], edge_start, SHARED_RADIUS)
                    else {
                        continue;
                    };
                    let Some((c, d)) = trim_segment(c, d, edge_end, SHARED_RADIUS) else {
                        continue;
                    };
                    if segment_crossing(a, b, c, d).is_some() {
                        penalty += CROSSING_PENALTY;
                    }
                }
            }
            penalty
        };

        for x in (0..520).step_by(17) {
            for y in (0..360).step_by(19) {
                let fx = x as f32;
                let fy = y as f32;
                for (a, b) in [([fx, fy], [fx + 45.0, fy]), ([fx, fy], [fx, fy + 45.0])] {
                    assert_eq!(
                        parallel_conflict(a, b, &routes, edge_start, edge_end),
                        parallel_conflict_indexed(a, b, &index, edge_start, edge_end),
                        "parallel mismatch at {a:?}-{b:?}"
                    );
                    assert_eq!(
                        scan_crossing(a, b),
                        crossing_penalty_indexed(a, b, &index, edge_start, edge_end),
                        "crossing mismatch at {a:?}-{b:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn example_routes_do_not_overlap() {
        for (_, key) in examples::EXAMPLES {
            let doc = examples::load_example(key).unwrap();
            check_routes(key, &doc, &layout(&doc));
            check_routes(
                key,
                &doc,
                &layout_with(
                    &doc,
                    &LayoutOptions {
                        route_style: RouteStyle::Orthogonal,
                    },
                ),
            );

            let mut auto = doc;
            auto_layout(&mut auto);
            check_routes(key, &auto, &layout(&auto));
            check_routes(
                key,
                &auto,
                &layout_with(
                    &auto,
                    &LayoutOptions {
                        route_style: RouteStyle::Orthogonal,
                    },
                ),
            );
        }
    }
}
