use crate::model::{Document, NodeId, NodeKind, Rect};
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

    let mut routes = HashMap::new();
    let mut routed: Vec<Vec<[f32; 2]>> = Vec::new();
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
        let points = if options.route_style == RouteStyle::Orthogonal {
            route_edge_orthogonal(&node_rects, &routed, start, [1.0, 0.0], end, end_dir)
                .unwrap_or_else(|| {
                    route_edge(&node_rects, &routed, start, [1.0, 0.0], end, end_dir)
                })
        } else {
            route_edge(&node_rects, &routed, start, [1.0, 0.0], end, end_dir)
        };
        let label_pos = route_label_pos(&points);
        routed.push(points.clone());
        routes.insert(
            edge.id,
            EdgeRoute {
                points,
                label_pos,
                feedback: end[0] < start[0] || end[1] < start[1] - 20.0,
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
const EDGE_CLEARANCE: f32 = 12.0;
const SHARED_RADIUS: f32 = 22.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum EdgeAvoidance {
    Hard,
    Soft,
    Off,
}

fn route_edge(
    node_rects: &HashMap<NodeId, Rect>,
    existing: &[Vec<[f32; 2]>],
    start: [f32; 2],
    start_dir: [f32; 2],
    end: [f32; 2],
    end_dir: [f32; 2],
) -> Vec<[f32; 2]> {
    let obstacles: Vec<Rect> = node_rects
        .values()
        .map(|rect| rect.expand(ROUTE_CLEARANCE))
        .collect();
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
        &obstacles,
        existing,
        start,
        end,
        EdgeAvoidance::Hard,
    )
    .or_else(|| {
        route_between(
            start_stub,
            end_stub,
            &obstacles,
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
            &obstacles,
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
    node_rects: &HashMap<NodeId, Rect>,
    existing: &[Vec<[f32; 2]>],
    start: [f32; 2],
    start_dir: [f32; 2],
    end: [f32; 2],
    end_dir: [f32; 2],
) -> Option<Vec<[f32; 2]>> {
    let obstacles: Vec<Rect> = node_rects
        .values()
        .map(|rect| rect.expand(ROUTE_CLEARANCE))
        .collect();
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
    let middle = orthogonal_route(
        start_stub,
        end_stub,
        &obstacles,
        existing,
        start,
        end,
        EdgeAvoidance::Hard,
    )
    .or_else(|| {
        orthogonal_route(
            start_stub,
            end_stub,
            &obstacles,
            existing,
            start,
            end,
            EdgeAvoidance::Soft,
        )
    })
    .or_else(|| {
        orthogonal_route(
            start_stub,
            end_stub,
            &obstacles,
            existing,
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

fn orthogonal_route(
    start: [f32; 2],
    end: [f32; 2],
    obstacles: &[Rect],
    existing: &[Vec<[f32; 2]>],
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
                existing,
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
                existing,
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
    simplify_orthogonal(
        &mut path, obstacles, existing, edge_start, edge_end, avoidance,
    );
    Some(path)
}

const BEND_PENALTY: f32 = 30.0;

fn orthogonal_penalty(
    a: [f32; 2],
    b: [f32; 2],
    obstacles: &[Rect],
    existing: &[Vec<[f32; 2]>],
    edge_start: [f32; 2],
    edge_end: [f32; 2],
    avoidance: EdgeAvoidance,
) -> Option<f32> {
    if !segment_clear(a, b, obstacles) {
        return None;
    }
    if parallel_conflict(a, b, existing, edge_start, edge_end) {
        match avoidance {
            EdgeAvoidance::Hard => None,
            EdgeAvoidance::Soft => Some(600.0),
            EdgeAvoidance::Off => Some(0.0),
        }
    } else {
        Some(0.0)
    }
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
    existing: &[Vec<[f32; 2]>],
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
            if axis_aligned && segment_ok(a, c, obstacles, existing, edge_start, edge_end, check) {
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

    let mut y_order: HashMap<NodeId, f32> = doc.nodes.iter().map(|n| (n.id, n.pos[1])).collect();
    for _ in 0..3 {
        for group in groups.iter_mut() {
            group.sort_by(|a, b| {
                let ay = neighbor_center(*a, doc, &y_order);
                let by = neighbor_center(*b, doc, &y_order);
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

    let max_rows = groups.iter().map(Vec::len).max().unwrap_or(1) as f32;
    let row_step = 130.0;
    let col_step = 320.0;
    for (li, group) in groups.iter().enumerate() {
        let total_h = (group.len().saturating_sub(1)) as f32 * row_step;
        let block_h = (max_rows - 1.0) * row_step;
        let start_y = 80.0 + (block_h - total_h) / 2.0;
        for (row, id) in group.iter().enumerate() {
            if let Some(node) = doc.node_mut(*id) {
                node.pos = [80.0 + li as f32 * col_step, start_y + row as f32 * row_step];
            }
        }
    }
}

fn neighbor_center(id: NodeId, doc: &Document, y_order: &HashMap<NodeId, f32>) -> f32 {
    let mut values = Vec::new();
    for edge in &doc.edges {
        if edge.to == id {
            if let Some(y) = y_order.get(&edge.from) {
                values.push(*y);
            }
        }
        if edge.from == id {
            if let Some(y) = y_order.get(&edge.to) {
                values.push(*y);
            }
        }
    }
    if values.is_empty() {
        y_order.get(&id).copied().unwrap_or(0.0)
    } else {
        values.iter().sum::<f32>() / values.len() as f32
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

    fn check_routes(key: &str, network: &NetworkLayout) {
        let mut routes: Vec<_> = network.edge_routes.iter().collect();
        routes.sort_by_key(|(id, _)| **id);
        for i in 0..routes.len() {
            for j in i + 1..routes.len() {
                let a_segments = trimmed_segments(&routes[i].1.points);
                let b_segments = trimmed_segments(&routes[j].1.points);
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
    fn example_routes_do_not_overlap() {
        for (_, key) in examples::EXAMPLES {
            let doc = examples::load_example(key).unwrap();
            check_routes(key, &layout(&doc));
            check_routes(
                key,
                &layout_with(
                    &doc,
                    &LayoutOptions {
                        route_style: RouteStyle::Orthogonal,
                    },
                ),
            );

            let mut auto = doc;
            auto_layout(&mut auto);
            check_routes(key, &layout(&auto));
            check_routes(
                key,
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
