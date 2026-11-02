use crate::model::{fit_text, text_width, Annotation, Rect};
use crate::scene::{Scene, TextAnchor, INK, NOTE_FILL};

const MAX_WIDTH: f32 = 220.0;
const PAD_X: f32 = 9.0;
const PAD_Y: f32 = 8.0;
const LINE_HEIGHT: f32 = 14.0;
const TEXT_SIZE: f32 = 11.0;

pub fn draw_annotation(
    scene: &mut Scene,
    annotation: &Annotation,
    anchor_rect: Option<Rect>,
    anchor_point: Option<[f32; 2]>,
    canvas: Option<Rect>,
) {
    let lines = wrap_lines(&annotation.text, MAX_WIDTH - PAD_X * 2.0, TEXT_SIZE);
    let text_width = lines
        .iter()
        .map(|line| text_width(line, TEXT_SIZE))
        .fold(0.0f32, f32::max);
    let box_width = (text_width + PAD_X * 2.0).clamp(70.0, MAX_WIDTH);
    let box_height = lines.len() as f32 * LINE_HEIGHT + PAD_Y * 2.0;
    let anchor = anchor_rect.map(|rect| rect.center()).or(anchor_point);
    let Some(anchor) = anchor else {
        return;
    };

    let mut position = annotation
        .position
        .unwrap_or([anchor[0] + 24.0, anchor[1] - box_height / 2.0]);
    if let Some(canvas) = canvas {
        position[0] = position[0].clamp(
            canvas.x + 8.0,
            (canvas.right() - box_width - 8.0).max(canvas.x + 8.0),
        );
        position[1] = position[1].clamp(
            canvas.y + 8.0,
            (canvas.bottom() - box_height - 8.0).max(canvas.y + 8.0),
        );
    }
    let rect = Rect::new(position[0], position[1], box_width, box_height);

    let target = match anchor_rect {
        Some(node_rect) => Some(nearest_boundary_point(node_rect, rect.center())),
        None => anchor_point,
    };
    if let Some(target) = target {
        if !rect.contains(target) {
            let base = nearest_boundary_point(rect, target);
            let direction = [target[0] - base[0], target[1] - base[1]];
            let length = (direction[0] * direction[0] + direction[1] * direction[1]).sqrt();
            if length > 1.0 {
                let half = 6.0;
                let normal = [-direction[1] / length * half, direction[0] / length * half];
                scene.path(
                    vec![
                        target,
                        [base[0] + normal[0], base[1] + normal[1]],
                        [base[0] - normal[0], base[1] - normal[1]],
                    ],
                    true,
                    NOTE_FILL,
                    None,
                );
            }
        }
    }

    scene.rect(rect, 4.0, NOTE_FILL, None);
    for (index, line) in lines.iter().enumerate() {
        scene.text(
            [
                rect.x + PAD_X,
                rect.y + PAD_Y + LINE_HEIGHT * 0.5 + index as f32 * LINE_HEIGHT,
            ],
            line,
            TEXT_SIZE,
            INK,
            TextAnchor::Start,
        );
    }
}

fn nearest_boundary_point(rect: Rect, point: [f32; 2]) -> [f32; 2] {
    let x = point[0].clamp(rect.x, rect.right());
    let y = point[1].clamp(rect.y, rect.bottom());
    if x > rect.x && x < rect.right() && y > rect.y && y < rect.bottom() {
        let left = x - rect.x;
        let right = rect.right() - x;
        let top = y - rect.y;
        let bottom = rect.bottom() - y;
        let minimum = left.min(right).min(top).min(bottom);
        if minimum == left {
            [rect.x, y]
        } else if minimum == right {
            [rect.right(), y]
        } else if minimum == top {
            [x, rect.y]
        } else {
            [x, rect.bottom()]
        }
    } else {
        [x, y]
    }
}

fn wrap_lines(text: &str, max_width: f32, size: f32) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.lines() {
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
