use crate::layout::{edge_direction, edge_tip, NetworkLayout};
use crate::model::{
    fit_text, text_width, AnnotationTarget, Document, NodeId, NodeKind, PortType, Rect,
};
use crate::scene::{
    arrow_head, Color, Scene, Stroke, TextAnchor, BLUE, CELL_INK, INK, MUTED, WHITE, YELLOW,
};

pub fn render_network(doc: &Document, layout: &NetworkLayout) -> Scene {
    let mut scene = Scene::new();

    // Component clouds sit behind the wires so a wire that grazes a cloud
    // lobe (or an arrow at a cloud port) is never swallowed by the fill.
    for node in &doc.nodes {
        if !node.kind.is_component() {
            continue;
        }
        let Some(rect) = layout.node_rects.get(&node.id).copied() else {
            continue;
        };
        scene.cloud(rect, WHITE, Stroke::new(INK, 1.6));
    }

    for edge in &doc.edges {
        let Some(route) = layout.edge_routes.get(&edge.id) else {
            continue;
        };
        let is_cell = doc
            .node(edge.from)
            .and_then(|node| node.kind.output_types().get(edge.from_port).copied())
            == Some(PortType::Cell);
        let color = if is_cell { CELL_INK } else { INK };
        let stroke = if is_cell {
            Stroke::dashed(color, 1.55, 6.0, 3.0)
        } else {
            Stroke::new(color, 1.55)
        };
        scene.polyline(route.points.clone(), stroke);
        if let Some(junction) = route.junction {
            scene.circle(junction, 2.6, color, None);
        }
        let tip = edge_tip(route);
        let dir = edge_direction(route);
        let head = arrow_head(tip, dir, 9.0);
        scene.path(head, true, color, None);

        if !edge.label.is_empty() {
            let size = 12.0;
            let max_width = edge.label_width.unwrap_or(220.0).max(60.0);
            let lines = wrap_label_lines(&edge.label, max_width, size);
            let line_height = size * 1.35;
            let width = lines
                .iter()
                .map(|line| text_width(line, size))
                .fold(0.0f32, f32::max);
            let height = lines.len() as f32 * line_height;
            let p = route.label_pos;
            scene.rect(
                Rect::new(
                    p[0] - width / 2.0 - 5.0,
                    p[1] - height / 2.0 - 3.0,
                    width + 10.0,
                    height + 6.0,
                ),
                3.0,
                WHITE,
                None,
            );
            let start_y = p[1] - height / 2.0 + line_height / 2.0;
            for (index, line) in lines.iter().enumerate() {
                scene.text(
                    [p[0], start_y + index as f32 * line_height],
                    line,
                    size,
                    MUTED,
                    TextAnchor::Center,
                );
            }
        }
    }

    for node in &doc.nodes {
        let Some(rect) = layout.node_rects.get(&node.id).copied() else {
            continue;
        };
        draw_node(&mut scene, doc, node.id, rect);
    }

    for annotation in &doc.annotations {
        match &annotation.target {
            Some(AnnotationTarget::Node { id }) => {
                if let Some(rect) = layout.node_rects.get(id).copied() {
                    crate::render_annotation::draw_annotation(
                        &mut scene,
                        annotation,
                        Some(rect),
                        None,
                        None,
                    );
                }
            }
            Some(AnnotationTarget::Edge { id }) => {
                if let Some(route) = layout.edge_routes.get(id) {
                    crate::render_annotation::draw_annotation(
                        &mut scene,
                        annotation,
                        None,
                        Some(route.label_pos),
                        None,
                    );
                }
            }
            Some(AnnotationTarget::Transaction { .. }) => {}
            None => {
                crate::render_annotation::draw_annotation(&mut scene, annotation, None, None, None)
            }
        }
    }

    scene
}

fn draw_node(scene: &mut Scene, doc: &Document, id: NodeId, rect: Rect) {
    let Some(node) = doc.node(id) else {
        return;
    };
    let shape_stroke = Stroke::new(INK, 1.6);

    match &node.kind {
        NodeKind::InputEvent { .. } | NodeKind::InputCell { .. } => {
            draw_boundary_label(scene, node, rect, true);
        }
        NodeKind::Sink => draw_boundary_label(scene, node, rect, false),
        NodeKind::Hold { .. } => {
            scene.rect(rect, 12.0, BLUE, Some(shape_stroke));
            scene.text(
                [rect.x + 12.0, rect.y - 13.0],
                node.kind.primitive(),
                13.5,
                INK,
                TextAnchor::Start,
            );
            scene.text(
                rect.center(),
                fit_text(&node.body_text(), rect.w - 24.0, 16.0),
                16.0,
                INK,
                TextAnchor::Center,
            );
        }
        NodeKind::Map { cell, .. } => {
            draw_chevron(scene, rect, if *cell { BLUE } else { YELLOW }, shape_stroke);
            draw_op_text(scene, node, rect, false);
        }
        NodeKind::Lift { .. } => {
            draw_chevron(scene, rect, BLUE, shape_stroke);
            draw_op_text(scene, node, rect, true);
        }
        NodeKind::Merge | NodeKind::OrElse => {
            let points = vec![
                [rect.x, rect.y],
                [rect.right(), rect.y + rect.h * 0.24],
                [rect.right(), rect.y + rect.h * 0.76],
                [rect.x, rect.bottom()],
            ];
            scene.path(points, true, YELLOW, Some(shape_stroke));
            draw_primitive_header(scene, node, rect);
            scene.text(
                [rect.x + 10.0, rect.y + rect.h * 0.30],
                "left",
                9.5,
                MUTED,
                TextAnchor::Start,
            );
            scene.text(
                [rect.x + 10.0, rect.y + rect.h * 0.70],
                "right",
                9.5,
                MUTED,
                TextAnchor::Start,
            );
        }
        NodeKind::Filter { .. } | NodeKind::Gate => {
            draw_diamond(scene, rect, YELLOW, shape_stroke);
            draw_filter_text(scene, node, rect);
        }
        NodeKind::Snapshot { .. } | NodeKind::Never => {
            draw_chevron(scene, rect, YELLOW, shape_stroke);
            draw_op_text(scene, node, rect, false);
        }
        NodeKind::Component { body, .. } => {
            let body_count = body.len() as f32;
            let name_y = rect.center()[1] - body_count * 7.0;
            scene.text(
                [rect.center()[0], name_y],
                fit_text(&node.name, rect.w - 34.0, 14.5),
                14.5,
                INK,
                TextAnchor::Center,
            );
            for (index, line) in body.iter().enumerate() {
                scene.text(
                    [rect.center()[0], name_y + 16.0 + index as f32 * 14.0],
                    fit_text(line, rect.w - 40.0, 10.5),
                    10.5,
                    MUTED,
                    TextAnchor::Center,
                );
            }
        }
    }

    let inputs = node.kind.input_types();
    for port in 0..inputs.len() {
        let center = crate::layout::input_port_pos(node, rect, port);
        let label = node.kind.port_label(port);
        if label.is_empty() || matches!(node.kind, NodeKind::Merge | NodeKind::OrElse) {
            continue;
        }
        let (pos, anchor) = match (&node.kind, port) {
            (NodeKind::Snapshot { .. } | NodeKind::Gate, 0) => {
                ([center[0] - 8.0, center[1] - 9.0], TextAnchor::End)
            }
            (NodeKind::Snapshot { .. } | NodeKind::Gate, 1) => {
                ([center[0] + 9.0, center[1] + 12.0], TextAnchor::Start)
            }
            (NodeKind::Hold { .. }, 0) => ([center[0] + 9.0, center[1] + 13.0], TextAnchor::Start),
            _ => ([rect.x + 10.0, center[1]], TextAnchor::Start),
        };
        scene.text(pos, label, 9.5, MUTED, anchor);
    }

    let outputs = node.kind.output_types();
    for port in 0..outputs.len() {
        let center = crate::layout::output_port_pos(node, rect, port);
        let label = node.kind.output_port_label(port);
        if !label.is_empty() {
            scene.text(
                [rect.right() - 10.0, center[1]],
                label,
                9.5,
                MUTED,
                TextAnchor::End,
            );
        }
    }
}

fn wrap_label_lines(text: &str, max_width: f32, size: f32) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let mut current = String::new();
        for word in paragraph.split_whitespace() {
            let candidate = if current.is_empty() {
                word.to_string()
            } else {
                format!("{current} {word}")
            };
            if !current.is_empty() && text_width(&candidate, size) > max_width {
                lines.push(std::mem::take(&mut current));
                current = word.to_string();
            } else {
                current = candidate;
            }
        }
        if !current.is_empty() {
            lines.push(current);
        }
    }
    if lines.is_empty() {
        lines.push(text.to_string());
    }
    lines
        .into_iter()
        .map(|line| fit_text(&line, max_width, size))
        .collect()
}

fn draw_boundary_label(scene: &mut Scene, node: &crate::model::Node, rect: Rect, input: bool) {
    let text = if node.name.is_empty() {
        node.kind.primitive()
    } else {
        node.name.as_str()
    };
    let text = fit_text(text, rect.w - 4.0, 13.5);
    let y = rect.center()[1];
    if input {
        scene.text([rect.right() - 4.0, y], text, 13.5, INK, TextAnchor::End);
    } else {
        scene.text([rect.x + 4.0, y], text, 13.5, INK, TextAnchor::Start);
    }
}

fn draw_primitive_header(scene: &mut Scene, node: &crate::model::Node, rect: Rect) {
    scene.text(
        [rect.center()[0], rect.y - 13.0],
        fit_text(node.kind.primitive(), rect.w + 20.0, 13.5),
        13.5,
        INK,
        TextAnchor::Center,
    );
}

fn draw_chevron(scene: &mut Scene, rect: Rect, fill: Color, stroke: Stroke) {
    let tip = (rect.h * 0.30).min(24.0);
    let points = vec![
        [rect.x, rect.y],
        [rect.right() - tip, rect.y],
        [rect.right(), rect.y + rect.h / 2.0],
        [rect.right() - tip, rect.bottom()],
        [rect.x, rect.bottom()],
    ];
    scene.path(points, true, fill, Some(stroke));
}

fn draw_diamond(scene: &mut Scene, rect: Rect, fill: Color, stroke: Stroke) {
    let points = vec![
        [rect.right(), rect.center()[1]],
        [rect.center()[0], rect.y],
        [rect.x, rect.center()[1]],
        [rect.center()[0], rect.bottom()],
    ];
    scene.path(points, true, fill, Some(stroke));
}

fn draw_op_text(scene: &mut Scene, node: &crate::model::Node, rect: Rect, has_second_label: bool) {
    draw_primitive_header(scene, node, rect);
    let body = node.body_text();
    if body.is_empty() {
        return;
    }
    let max_w = rect.w - if has_second_label { 64.0 } else { 42.0 };
    let x = if has_second_label {
        rect.x + rect.w * 0.56
    } else {
        rect.center()[0] + 4.0
    };
    scene.text(
        [x, rect.center()[1]],
        fit_text(&body, max_w, 14.0),
        14.0,
        INK,
        TextAnchor::Center,
    );
}

fn draw_filter_text(scene: &mut Scene, node: &crate::model::Node, rect: Rect) {
    draw_primitive_header(scene, node, rect);
    let body = node.body_text();
    if body.is_empty() {
        return;
    }
    let size = 11.5;
    let lines = wrap_lines(&body, rect.w * 0.64, size);
    let center = rect.center();
    let line_height = size * 1.35;
    let start_y = center[1] - (lines.len().saturating_sub(1) as f32 * line_height) / 2.0;
    for (index, line) in lines.iter().enumerate() {
        scene.text(
            [center[0], start_y + index as f32 * line_height],
            line,
            size,
            INK,
            TextAnchor::Center,
        );
    }
}

fn wrap_lines(text: &str, max_width: f32, size: f32) -> Vec<String> {
    if text_width(text, size) <= max_width {
        return vec![text.to_string()];
    }
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let candidate = if current.is_empty() {
            word.to_string()
        } else {
            format!("{current} {word}")
        };
        if !current.is_empty() && text_width(&candidate, size) > max_width {
            lines.push(current);
            current = word.to_string();
        } else {
            current = candidate;
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.len() > 2 {
        let rest = lines.split_off(2).join(" ");
        lines.push(rest);
    }
    lines
        .into_iter()
        .map(|line| fit_text(&line, max_width, size))
        .collect()
}

pub fn draw_notation_key(scene: &mut Scene, x: f32, y: f32) -> f32 {
    let mut cursor = x;
    cursor = key_swatch(scene, cursor, y, YELLOW, "stream") + 14.0;
    cursor = key_swatch(scene, cursor, y, BLUE, "cell") + 14.0;
    cursor = key_line(scene, cursor, y, Stroke::new(INK, 1.55), "stream wire") + 14.0;
    cursor = key_line(
        scene,
        cursor,
        y,
        Stroke::dashed(CELL_INK, 1.55, 6.0, 3.0),
        "cell wire",
    );
    cursor - x
}

fn key_line(scene: &mut Scene, x: f32, y: f32, stroke: Stroke, label: &str) -> f32 {
    let size = 10.5;
    scene.line([x, y], [x + 22.0, y], stroke);
    let text_x = x + 28.0;
    scene.text([text_x, y], label, size, MUTED, TextAnchor::Start);
    text_x + text_width(label, size)
}

fn key_swatch(scene: &mut Scene, x: f32, y: f32, fill: Color, label: &str) -> f32 {
    let size = 10.5;
    scene.rect(
        Rect::new(x, y - 5.0, 18.0, 10.0),
        3.0,
        fill,
        Some(Stroke::new(INK, 1.0)),
    );
    let text_x = x + 25.0;
    scene.text([text_x, y], label, size, MUTED, TextAnchor::Start);
    text_x + text_width(label, size)
}
