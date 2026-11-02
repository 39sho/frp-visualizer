use crate::model::{text_width, Rect};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color(pub u8, pub u8, pub u8, pub u8);

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self(r, g, b, 255)
    }

    pub fn to_css(self) -> String {
        if self.3 == 255 {
            format!("#{:02x}{:02x}{:02x}", self.0, self.1, self.2)
        } else {
            format!(
                "rgba({},{},{},{})",
                self.0,
                self.1,
                self.2,
                self.3 as f32 / 255.0
            )
        }
    }
}

pub const INK: Color = Color::rgb(24, 24, 24);
pub const MUTED: Color = Color::rgb(105, 105, 105);
pub const FAINT: Color = Color::rgb(176, 176, 176);
pub const WHITE: Color = Color::rgb(255, 255, 255);
pub const YELLOW: Color = Color::rgb(249, 246, 176);
pub const BLUE: Color = Color::rgb(207, 230, 245);
pub const CELL_INK: Color = Color::rgb(74, 121, 158);
pub const ACCENT: Color = Color::rgb(45, 108, 223);
pub const ACCENT_SOFT: Color = Color::rgb(224, 235, 255);
pub const NOTE_FILL: Color = Color::rgb(154, 198, 88);

#[derive(Clone, Copy, Debug)]
pub struct Stroke {
    pub color: Color,
    pub width: f32,
    pub dash: Option<[f32; 2]>,
}

impl Stroke {
    pub fn new(color: Color, width: f32) -> Self {
        Self {
            color,
            width,
            dash: None,
        }
    }

    pub fn dashed(color: Color, width: f32, dash: f32, gap: f32) -> Self {
        Self {
            color,
            width,
            dash: Some([dash, gap]),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextAnchor {
    Start,
    Center,
    End,
}

#[derive(Clone, Debug)]
pub enum Shape {
    Rect {
        rect: Rect,
        radius: f32,
        fill: Color,
        stroke: Option<Stroke>,
    },
    Path {
        points: Vec<[f32; 2]>,
        closed: bool,
        fill: Color,
        stroke: Option<Stroke>,
    },
    Polyline {
        points: Vec<[f32; 2]>,
        stroke: Stroke,
    },
    Line {
        a: [f32; 2],
        b: [f32; 2],
        stroke: Stroke,
    },
    Circle {
        center: [f32; 2],
        radius: f32,
        fill: Color,
        stroke: Option<Stroke>,
    },
    Cloud {
        rect: Rect,
        fill: Color,
        stroke: Stroke,
    },
    Text {
        pos: [f32; 2],
        text: String,
        size: f32,
        color: Color,
        anchor: TextAnchor,
        bold: bool,
    },
}

#[derive(Clone, Debug, Default)]
pub struct Scene {
    pub shapes: Vec<Shape>,
}

impl Scene {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn rect(&mut self, rect: Rect, radius: f32, fill: Color, stroke: Option<Stroke>) {
        self.shapes.push(Shape::Rect {
            rect,
            radius,
            fill,
            stroke,
        });
    }

    pub fn path(
        &mut self,
        points: Vec<[f32; 2]>,
        closed: bool,
        fill: Color,
        stroke: Option<Stroke>,
    ) {
        self.shapes.push(Shape::Path {
            points,
            closed,
            fill,
            stroke,
        });
    }

    pub fn polyline(&mut self, points: Vec<[f32; 2]>, stroke: Stroke) {
        self.shapes.push(Shape::Polyline { points, stroke });
    }

    pub fn line(&mut self, a: [f32; 2], b: [f32; 2], stroke: Stroke) {
        self.shapes.push(Shape::Line { a, b, stroke });
    }

    pub fn circle(&mut self, center: [f32; 2], radius: f32, fill: Color, stroke: Option<Stroke>) {
        self.shapes.push(Shape::Circle {
            center,
            radius,
            fill,
            stroke,
        });
    }

    pub fn cloud(&mut self, rect: Rect, fill: Color, stroke: Stroke) {
        self.shapes.push(Shape::Cloud { rect, fill, stroke });
    }

    pub fn text(
        &mut self,
        pos: [f32; 2],
        text: impl Into<String>,
        size: f32,
        color: Color,
        anchor: TextAnchor,
    ) {
        self.shapes.push(Shape::Text {
            pos,
            text: text.into(),
            size,
            color,
            anchor,
            bold: false,
        });
    }

    pub fn bold_text(
        &mut self,
        pos: [f32; 2],
        text: impl Into<String>,
        size: f32,
        color: Color,
        anchor: TextAnchor,
    ) {
        self.shapes.push(Shape::Text {
            pos,
            text: text.into(),
            size,
            color,
            anchor,
            bold: true,
        });
    }

    pub fn append(&mut self, other: &Scene, dx: f32, dy: f32) {
        for shape in &other.shapes {
            self.shapes.push(translate_shape(shape, dx, dy));
        }
    }

    pub fn append_scaled(&mut self, other: &Scene, dx: f32, dy: f32, scale: f32) {
        for shape in &other.shapes {
            self.shapes.push(scale_shape(shape, dx, dy, scale));
        }
    }

    pub fn translate(&mut self, dx: f32, dy: f32) {
        for shape in &mut self.shapes {
            *shape = translate_shape(shape, dx, dy);
        }
    }

    pub fn bounds(&self) -> Option<Rect> {
        let mut min_x = f32::INFINITY;
        let mut min_y = f32::INFINITY;
        let mut max_x = f32::NEG_INFINITY;
        let mut max_y = f32::NEG_INFINITY;
        let mut add = |x0: f32, y0: f32, x1: f32, y1: f32| {
            min_x = min_x.min(x0);
            min_y = min_y.min(y0);
            max_x = max_x.max(x1);
            max_y = max_y.max(y1);
        };
        for shape in &self.shapes {
            match shape {
                Shape::Rect { rect, .. } | Shape::Cloud { rect, .. } => {
                    add(rect.x, rect.y, rect.right(), rect.bottom())
                }
                Shape::Path { points, .. } | Shape::Polyline { points, .. } => {
                    for p in points {
                        add(p[0], p[1], p[0], p[1]);
                    }
                }
                Shape::Line { a, b, .. } => {
                    add(a[0], a[1], b[0], b[1]);
                }
                Shape::Circle { center, radius, .. } => add(
                    center[0] - radius,
                    center[1] - radius,
                    center[0] + radius,
                    center[1] + radius,
                ),
                Shape::Text {
                    pos,
                    text,
                    size,
                    anchor,
                    ..
                } => {
                    let w = text_width(text, *size);
                    let (x0, x1) = match anchor {
                        TextAnchor::Start => (pos[0], pos[0] + w),
                        TextAnchor::Center => (pos[0] - w / 2.0, pos[0] + w / 2.0),
                        TextAnchor::End => (pos[0] - w, pos[0]),
                    };
                    add(x0, pos[1] - size * 0.6, x1, pos[1] + size * 0.6);
                }
            }
        }
        if min_x.is_finite() {
            Some(Rect::new(min_x, min_y, max_x - min_x, max_y - min_y))
        } else {
            None
        }
    }

    pub fn to_svg(&self, width: f32, height: f32, margin: f32) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}">"#,
            w = (width + margin * 2.0).ceil(),
            h = (height + margin * 2.0).ceil()
        ));
        out.push('\n');
        out.push_str(&format!(
            r#"<rect width="100%" height="100%" fill="{}"/>"#,
            WHITE.to_css()
        ));
        out.push('\n');
        out.push_str(&format!(r#"<g transform="translate({margin},{margin})">"#));
        out.push('\n');
        for shape in &self.shapes {
            shape_to_svg(shape, &mut out);
        }
        out.push_str("</g>\n</svg>\n");
        out
    }
}

fn translate_point(p: [f32; 2], dx: f32, dy: f32) -> [f32; 2] {
    [p[0] + dx, p[1] + dy]
}

fn scale_point(p: [f32; 2], dx: f32, dy: f32, scale: f32) -> [f32; 2] {
    [p[0] * scale + dx, p[1] * scale + dy]
}

fn scale_stroke(stroke: &Stroke, scale: f32) -> Stroke {
    let mut scaled = *stroke;
    scaled.width *= scale;
    if let Some(dash) = &mut scaled.dash {
        dash[0] *= scale;
        dash[1] *= scale;
    }
    scaled
}

fn scale_shape(shape: &Shape, dx: f32, dy: f32, scale: f32) -> Shape {
    match shape {
        Shape::Rect {
            rect,
            radius,
            fill,
            stroke,
        } => Shape::Rect {
            rect: Rect::new(
                rect.x * scale + dx,
                rect.y * scale + dy,
                rect.w * scale,
                rect.h * scale,
            ),
            radius: radius * scale,
            fill: *fill,
            stroke: stroke.map(|stroke| scale_stroke(&stroke, scale)),
        },
        Shape::Path {
            points,
            closed,
            fill,
            stroke,
        } => Shape::Path {
            points: points
                .iter()
                .map(|p| scale_point(*p, dx, dy, scale))
                .collect(),
            closed: *closed,
            fill: *fill,
            stroke: stroke.map(|stroke| scale_stroke(&stroke, scale)),
        },
        Shape::Polyline { points, stroke } => Shape::Polyline {
            points: points
                .iter()
                .map(|p| scale_point(*p, dx, dy, scale))
                .collect(),
            stroke: scale_stroke(stroke, scale),
        },
        Shape::Line { a, b, stroke } => Shape::Line {
            a: scale_point(*a, dx, dy, scale),
            b: scale_point(*b, dx, dy, scale),
            stroke: scale_stroke(stroke, scale),
        },
        Shape::Circle {
            center,
            radius,
            fill,
            stroke,
        } => Shape::Circle {
            center: scale_point(*center, dx, dy, scale),
            radius: radius * scale,
            fill: *fill,
            stroke: stroke.map(|stroke| scale_stroke(&stroke, scale)),
        },
        Shape::Cloud { rect, fill, stroke } => Shape::Cloud {
            rect: Rect::new(
                rect.x * scale + dx,
                rect.y * scale + dy,
                rect.w * scale,
                rect.h * scale,
            ),
            fill: *fill,
            stroke: scale_stroke(stroke, scale),
        },
        Shape::Text {
            pos,
            text,
            size,
            color,
            anchor,
            bold,
        } => Shape::Text {
            pos: scale_point(*pos, dx, dy, scale),
            text: text.clone(),
            size: size * scale,
            color: *color,
            anchor: *anchor,
            bold: *bold,
        },
    }
}

fn translate_shape(shape: &Shape, dx: f32, dy: f32) -> Shape {
    match shape {
        Shape::Rect {
            rect,
            radius,
            fill,
            stroke,
        } => Shape::Rect {
            rect: Rect::new(rect.x + dx, rect.y + dy, rect.w, rect.h),
            radius: *radius,
            fill: *fill,
            stroke: *stroke,
        },
        Shape::Path {
            points,
            closed,
            fill,
            stroke,
        } => Shape::Path {
            points: points.iter().map(|p| translate_point(*p, dx, dy)).collect(),
            closed: *closed,
            fill: *fill,
            stroke: *stroke,
        },
        Shape::Polyline { points, stroke } => Shape::Polyline {
            points: points.iter().map(|p| translate_point(*p, dx, dy)).collect(),
            stroke: *stroke,
        },
        Shape::Line { a, b, stroke } => Shape::Line {
            a: translate_point(*a, dx, dy),
            b: translate_point(*b, dx, dy),
            stroke: *stroke,
        },
        Shape::Circle {
            center,
            radius,
            fill,
            stroke,
        } => Shape::Circle {
            center: translate_point(*center, dx, dy),
            radius: *radius,
            fill: *fill,
            stroke: *stroke,
        },
        Shape::Cloud { rect, fill, stroke } => Shape::Cloud {
            rect: Rect::new(rect.x + dx, rect.y + dy, rect.w, rect.h),
            fill: *fill,
            stroke: *stroke,
        },
        Shape::Text {
            pos,
            text,
            size,
            color,
            anchor,
            bold,
        } => Shape::Text {
            pos: translate_point(*pos, dx, dy),
            text: text.clone(),
            size: *size,
            color: *color,
            anchor: *anchor,
            bold: *bold,
        },
    }
}

fn shape_to_svg(shape: &Shape, out: &mut String) {
    match shape {
        Shape::Rect {
            rect,
            radius,
            fill,
            stroke,
        } => {
            out.push_str(&format!(
                r#"<rect x="{:.2}" y="{:.2}" width="{:.2}" height="{:.2}" rx="{:.2}" fill="{}""#,
                rect.x,
                rect.y,
                rect.w,
                rect.h,
                radius,
                fill.to_css()
            ));
            if let Some(stroke) = stroke {
                out.push_str(&format!(
                    r#" stroke="{}" stroke-width="{:.2}""#,
                    stroke.color.to_css(),
                    stroke.width
                ));
                if let Some([dash, gap]) = stroke.dash {
                    out.push_str(&format!(r#" stroke-dasharray="{dash:.2} {gap:.2}""#));
                }
            }
            out.push_str("/>\n");
        }
        Shape::Path {
            points,
            closed,
            fill,
            stroke,
        } => {
            if points.is_empty() {
                return;
            }
            out.push_str(&format!(
                r#"<path d="{}" fill="{}""#,
                path_data(points, *closed),
                fill.to_css()
            ));
            if let Some(stroke) = stroke {
                out.push_str(&format!(
                    r#" stroke="{}" stroke-width="{:.2}" stroke-linejoin="round""#,
                    stroke.color.to_css(),
                    stroke.width
                ));
                if let Some([dash, gap]) = stroke.dash {
                    out.push_str(&format!(r#" stroke-dasharray="{dash:.2} {gap:.2}""#));
                }
            }
            out.push_str("/>\n");
        }
        Shape::Polyline { points, stroke } => {
            if points.is_empty() {
                return;
            }
            out.push_str(&format!(
                r#"<path d="{}" fill="none" stroke="{}" stroke-width="{:.2}" stroke-linecap="round" stroke-linejoin="round""#,
                path_data(points, false),
                stroke.color.to_css(),
                stroke.width
            ));
            if let Some([dash, gap]) = stroke.dash {
                out.push_str(&format!(r#" stroke-dasharray="{dash:.2} {gap:.2}""#));
            }
            out.push_str("/>\n");
        }
        Shape::Line { a, b, stroke } => {
            out.push_str(&format!(
                r#"<line x1="{:.2}" y1="{:.2}" x2="{:.2}" y2="{:.2}" stroke="{}" stroke-width="{:.2}" stroke-linecap="round""#,
                a[0],
                a[1],
                b[0],
                b[1],
                stroke.color.to_css(),
                stroke.width
            ));
            if let Some([dash, gap]) = stroke.dash {
                out.push_str(&format!(r#" stroke-dasharray="{dash:.2} {gap:.2}""#));
            }
            out.push_str("/>\n");
        }
        Shape::Circle {
            center,
            radius,
            fill,
            stroke,
        } => {
            out.push_str(&format!(
                r#"<circle cx="{:.2}" cy="{:.2}" r="{:.2}" fill="{}""#,
                center[0],
                center[1],
                radius,
                fill.to_css()
            ));
            if let Some(stroke) = stroke {
                out.push_str(&format!(
                    r#" stroke="{}" stroke-width="{:.2}""#,
                    stroke.color.to_css(),
                    stroke.width
                ));
                if let Some([dash, gap]) = stroke.dash {
                    out.push_str(&format!(r#" stroke-dasharray="{dash:.2} {gap:.2}""#));
                }
            }
            out.push_str("/>\n");
        }
        Shape::Cloud { rect, fill, stroke } => {
            let points = cloud_points(*rect);
            out.push_str(&format!(
                r#"<path d="{}" fill="{}" stroke="{}" stroke-width="{:.2}" stroke-linejoin="round"/>"#,
                path_data(&points, true),
                fill.to_css(),
                stroke.color.to_css(),
                stroke.width
            ));
            out.push('\n');
        }
        Shape::Text {
            pos,
            text,
            size,
            color,
            anchor,
            bold,
        } => {
            let anchor_attr = match anchor {
                TextAnchor::Start => "start",
                TextAnchor::Center => "middle",
                TextAnchor::End => "end",
            };
            out.push_str(&format!(
                r#"<text x="{:.2}" y="{:.2}" font-family="Helvetica, Arial, 'Noto Sans JP', sans-serif" font-size="{:.2}" fill="{}" text-anchor="{}" dominant-baseline="central"{}>{}</text>"#,
                pos[0],
                pos[1],
                size,
                color.to_css(),
                anchor_attr,
                if *bold {
                    r#" font-weight="600""#
                } else {
                    ""
                },
                escape_xml(text)
            ));
            out.push('\n');
        }
    }
}

fn path_data(points: &[[f32; 2]], closed: bool) -> String {
    let mut s = String::new();
    for (i, p) in points.iter().enumerate() {
        if i == 0 {
            s.push_str(&format!("M{:.2},{:.2}", p[0], p[1]));
        } else {
            s.push_str(&format!(" L{:.2},{:.2}", p[0], p[1]));
        }
    }
    if closed {
        s.push_str(" Z");
    }
    s
}

fn escape_xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub fn cloud_points(rect: Rect) -> Vec<[f32; 2]> {
    let cx = rect.x + rect.w / 2.0;
    let cy = rect.y + rect.h / 2.0;
    let rx = rect.w / 2.0;
    let ry = rect.h / 2.0;
    let lobes = 9;
    let steps = 9;
    let mut points = Vec::with_capacity(lobes * steps);
    for k in 0..lobes {
        let a0 = std::f32::consts::TAU * k as f32 / lobes as f32;
        let a1 = std::f32::consts::TAU * (k + 1) as f32 / lobes as f32;
        let p0 = [cx + rx * a0.cos(), cy + ry * a0.sin()];
        let p1 = [cx + rx * a1.cos(), cy + ry * a1.sin()];
        let chord = ((p1[0] - p0[0]).powi(2) + (p1[1] - p0[1]).powi(2)).sqrt();
        let mid = [(p0[0] + p1[0]) / 2.0, (p0[1] + p1[1]) / 2.0];
        let mut nx = mid[0] - cx;
        let mut ny = mid[1] - cy;
        let nl = (nx * nx + ny * ny).sqrt().max(1e-3);
        nx /= nl;
        ny /= nl;
        let control = [mid[0] + nx * chord * 0.62, mid[1] + ny * chord * 0.62];
        for s in 0..steps {
            let u = s as f32 / steps as f32;
            let v = 1.0 - u;
            points.push([
                v * v * p0[0] + 2.0 * v * u * control[0] + u * u * p1[0],
                v * v * p0[1] + 2.0 * v * u * control[1] + u * u * p1[1],
            ]);
        }
    }
    points
}

pub fn arrow_head(tip: [f32; 2], direction: [f32; 2], size: f32) -> Vec<[f32; 2]> {
    let len = (direction[0] * direction[0] + direction[1] * direction[1]).sqrt();
    if len < 1e-5 {
        return vec![tip, tip, tip];
    }
    let ux = direction[0] / len;
    let uy = direction[1] / len;
    let back = [tip[0] - ux * size, tip[1] - uy * size];
    let nx = -uy * size * 0.45;
    let ny = ux * size * 0.45;
    vec![
        tip,
        [back[0] + nx, back[1] + ny],
        [back[0] - nx, back[1] - ny],
    ]
}

pub fn rounded_polyline(points: &[[f32; 2]], radius: f32) -> Vec<[f32; 2]> {
    if points.len() < 3 {
        return points.to_vec();
    }
    let mut out = vec![points[0]];
    for i in 1..points.len() - 1 {
        let prev = points[i - 1];
        let cur = points[i];
        let next = points[i + 1];
        let v1 = [prev[0] - cur[0], prev[1] - cur[1]];
        let v2 = [next[0] - cur[0], next[1] - cur[1]];
        let l1 = (v1[0] * v1[0] + v1[1] * v1[1]).sqrt();
        let l2 = (v2[0] * v2[0] + v2[1] * v2[1]).sqrt();
        if l1 < 1e-4 || l2 < 1e-4 {
            out.push(cur);
            continue;
        }
        let r = radius.min(l1 / 2.0).min(l2 / 2.0);
        let a = [cur[0] + v1[0] / l1 * r, cur[1] + v1[1] / l1 * r];
        let b = [cur[0] + v2[0] / l2 * r, cur[1] + v2[1] / l2 * r];
        out.push(a);
        for s in 1..=6 {
            let u = s as f32 / 6.0;
            let vu = 1.0 - u;
            out.push([
                vu * vu * a[0] + 2.0 * vu * u * cur[0] + u * u * b[0],
                vu * vu * a[1] + 2.0 * vu * u * cur[1] + u * u * b[1],
            ]);
        }
        out.push(b);
    }
    out.push(*points.last().unwrap());
    out
}

pub fn point_distance_to_polyline(p: [f32; 2], points: &[[f32; 2]]) -> f32 {
    let mut best = f32::INFINITY;
    for w in points.windows(2) {
        let a = w[0];
        let b = w[1];
        let ab = [b[0] - a[0], b[1] - a[1]];
        let ap = [p[0] - a[0], p[1] - a[1]];
        let len2 = ab[0] * ab[0] + ab[1] * ab[1];
        let t = if len2 < 1e-6 {
            0.0
        } else {
            ((ap[0] * ab[0] + ap[1] * ab[1]) / len2).clamp(0.0, 1.0)
        };
        let q = [a[0] + ab[0] * t, a[1] + ab[1] * t];
        let d = ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt();
        best = best.min(d);
    }
    best
}
