use serde::{Deserialize, Serialize};
use std::fmt;

pub type NodeId = u64;

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Value {
    #[default]
    Unit,
    Bool(bool),
    Number(f64),
    Text(String),
}

impl Value {
    pub fn parse(input: &str) -> Self {
        let s = input.trim();
        if s.is_empty() || s == "()" || s.eq_ignore_ascii_case("unit") {
            return Self::Unit;
        }
        if s.eq_ignore_ascii_case("true") {
            return Self::Bool(true);
        }
        if s.eq_ignore_ascii_case("false") {
            return Self::Bool(false);
        }
        if let Ok(n) = s.parse::<f64>() {
            if n.is_finite() {
                return Self::Number(n);
            }
        }
        Self::Text(s.to_string())
    }

    pub fn as_number(&self) -> Option<f64> {
        match self {
            Self::Number(n) => Some(*n),
            Self::Text(s) => s.trim().parse::<f64>().ok(),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            Self::Number(n) => Some(*n != 0.0),
            Self::Text(s) if s.eq_ignore_ascii_case("true") => Some(true),
            Self::Text(s) if s.eq_ignore_ascii_case("false") => Some(false),
            _ => None,
        }
    }

    pub fn short(&self, max: usize) -> String {
        let s = self.to_string();
        if s.chars().count() <= max {
            s
        } else {
            let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
            out.push('…');
            out
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unit => write!(f, "()"),
            Self::Bool(b) => write!(f, "{}", b),
            Self::Number(n) => {
                if (n.fract()).abs() < 1e-9 {
                    write!(f, "{}", *n as i64)
                } else {
                    write!(f, "{}", (n * 1000.0).round() / 1000.0)
                }
            }
            Self::Text(s) => write!(f, "{}", s),
        }
    }
}

impl Serialize for Value {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Value {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Ok(Self::parse(&s))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TimedValue {
    pub time: f64,
    pub value: Value,
}

impl TimedValue {
    pub fn new(time: f64, value: Value) -> Self {
        Self { time, value }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(tag = "op", content = "value", rename_all = "snake_case")]
pub enum UnaryOp {
    #[default]
    Identity,
    Add(f64),
    Multiply(f64),
    Negate,
    Reverse,
    Upper,
    Lower,
    Length,
    ToText,
    Not,
    Constant(f64),
}

impl UnaryOp {
    pub fn label(&self) -> String {
        match self {
            Self::Identity => "x".into(),
            Self::Add(n) => format!("x + {}", fmt_num(*n)),
            Self::Multiply(n) => format!("x × {}", fmt_num(*n)),
            Self::Negate => "−x".into(),
            Self::Reverse => "reverse(x)".into(),
            Self::Upper => "upper(x)".into(),
            Self::Lower => "lower(x)".into(),
            Self::Length => "len(x)".into(),
            Self::ToText => "str(x)".into(),
            Self::Not => "¬x".into(),
            Self::Constant(n) => format!("→ {}", fmt_num(*n)),
        }
    }

    pub fn apply(&self, v: &Value) -> Value {
        match self {
            Self::Identity => v.clone(),
            Self::Add(n) => match v.as_number() {
                Some(x) => Value::Number(x + n),
                None => Value::Unit,
            },
            Self::Multiply(n) => match v.as_number() {
                Some(x) => Value::Number(x * n),
                None => Value::Unit,
            },
            Self::Negate => match v.as_number() {
                Some(x) => Value::Number(-x),
                None => Value::Unit,
            },
            Self::Reverse => match v {
                Value::Text(s) => Value::Text(s.chars().rev().collect()),
                other => Value::Text(other.to_string().chars().rev().collect()),
            },
            Self::Upper => Value::Text(v.to_string().to_uppercase()),
            Self::Lower => Value::Text(v.to_string().to_lowercase()),
            Self::Length => Value::Number(v.to_string().chars().count() as f64),
            Self::ToText => Value::Text(v.to_string()),
            Self::Not => match v.as_bool() {
                Some(b) => Value::Bool(!b),
                None => Value::Unit,
            },
            Self::Constant(n) => Value::Number(*n),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(tag = "op", content = "value", rename_all = "snake_case")]
pub enum FilterOp {
    #[default]
    Always,
    NonZero,
    Positive,
    NonEmpty,
    GreaterThan(f64),
    Equals(String),
    Contains(String),
    Not,
}

impl FilterOp {
    pub fn label(&self) -> String {
        match self {
            Self::Always => "always".into(),
            Self::NonZero => "x ≠ 0".into(),
            Self::Positive => "x > 0".into(),
            Self::NonEmpty => "len(x) > 0".into(),
            Self::GreaterThan(n) => format!("x > {}", fmt_num(*n)),
            Self::Equals(s) => format!("x = {}", s),
            Self::Contains(s) => format!("x ∋ {}", s),
            Self::Not => "¬x".into(),
        }
    }

    pub fn test(&self, v: &Value) -> bool {
        match self {
            Self::Always => true,
            Self::NonZero => v.as_number().map(|n| n != 0.0).unwrap_or(false),
            Self::Positive => v.as_number().map(|n| n > 0.0).unwrap_or(false),
            Self::NonEmpty => !v.to_string().is_empty(),
            Self::GreaterThan(n) => v.as_number().map(|x| x > *n).unwrap_or(false),
            Self::Equals(s) => v.to_string() == *s,
            Self::Contains(s) => v.to_string().contains(s),
            Self::Not => !v.as_bool().unwrap_or(false),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(tag = "op", content = "value", rename_all = "snake_case")]
pub enum CombineOp {
    #[default]
    Add,
    Subtract,
    Multiply,
    Concat,
    Pair,
    KeepLeft,
    KeepRight,
    Max,
    Min,
}

impl CombineOp {
    pub fn label(&self) -> String {
        match self {
            Self::Add => "x + y".into(),
            Self::Subtract => "x − y".into(),
            Self::Multiply => "x × y".into(),
            Self::Concat => "x y".into(),
            Self::Pair => "(x, y)".into(),
            Self::KeepLeft => "x".into(),
            Self::KeepRight => "y".into(),
            Self::Max => "max(x, y)".into(),
            Self::Min => "min(x, y)".into(),
        }
    }

    pub fn symbol(&self) -> &'static str {
        match self {
            Self::Add => "+",
            Self::Subtract => "−",
            Self::Multiply => "×",
            Self::Concat => "‖",
            Self::Pair => "(·,·)",
            Self::KeepLeft => "x",
            Self::KeepRight => "y",
            Self::Max => "max",
            Self::Min => "min",
        }
    }

    pub fn apply(&self, a: &Value, b: &Value) -> Value {
        match self {
            Self::Add => num2(a, b, |x, y| x + y),
            Self::Subtract => num2(a, b, |x, y| x - y),
            Self::Multiply => num2(a, b, |x, y| x * y),
            Self::Concat => {
                let left = a.to_string();
                let right = b.to_string();
                if left.is_empty() {
                    Value::Text(right)
                } else if right.is_empty() {
                    Value::Text(left)
                } else {
                    Value::Text(format!("{left} {right}"))
                }
            }
            Self::Pair => Value::Text(format!("({a}, {b})")),
            Self::KeepLeft => a.clone(),
            Self::KeepRight => b.clone(),
            Self::Max => num2(a, b, f64::max),
            Self::Min => num2(a, b, f64::min),
        }
    }
}

fn num2(a: &Value, b: &Value, f: impl Fn(f64, f64) -> f64) -> Value {
    match (a.as_number(), b.as_number()) {
        (Some(x), Some(y)) => Value::Number(f(x, y)),
        _ => Value::Unit,
    }
}

fn fmt_num(n: f64) -> String {
    if (n.fract()).abs() < 1e-9 {
        format!("{}", n as i64)
    } else {
        format!("{}", (n * 1000.0).round() / 1000.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortType {
    Stream,
    Cell,
    Any,
}

fn default_port_type() -> PortType {
    PortType::Stream
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PortSpec {
    pub name: String,
    #[serde(default = "default_port_type")]
    pub ty: PortType,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentMode {
    #[default]
    Executable,
    Structural,
}

fn is_executable(mode: &DocumentMode) -> bool {
    *mode == DocumentMode::Executable
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AnnotationTarget {
    Node { id: NodeId },
    Edge { id: u64 },
    Transaction { index: usize },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Annotation {
    pub text: String,
    #[serde(default)]
    pub target: Option<AnnotationTarget>,
    #[serde(default)]
    pub position: Option<[f32; 2]>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ComponentDef {
    pub name: String,
    #[serde(default)]
    pub inputs: Vec<PortSpec>,
    #[serde(default)]
    pub outputs: Vec<PortSpec>,
    #[serde(default)]
    pub body: Vec<String>,
    pub doc: Box<Document>,
}

impl PortType {
    pub fn accepts(self, source: PortType) -> bool {
        self == PortType::Any || source == self
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Stream => "stream",
            Self::Cell => "cell",
            Self::Any => "any",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NodeKind {
    InputEvent {
        #[serde(default)]
        events: Vec<TimedValue>,
    },
    InputCell {
        #[serde(default)]
        initial: Value,
        #[serde(default)]
        changes: Vec<TimedValue>,
    },
    Map {
        #[serde(default)]
        op: UnaryOp,
    },
    Filter {
        #[serde(default)]
        op: FilterOp,
    },
    Merge,
    Snapshot {
        #[serde(default)]
        op: CombineOp,
    },
    Hold {
        #[serde(default)]
        initial: Value,
    },
    Lift {
        #[serde(default)]
        op: CombineOp,
    },
    Never,
    Sink,
    Component {
        #[serde(default)]
        definition: Option<String>,
        #[serde(default)]
        inputs: Vec<PortSpec>,
        #[serde(default)]
        outputs: Vec<PortSpec>,
        #[serde(default)]
        body: Vec<String>,
    },
}

impl NodeKind {
    pub fn primitive(&self) -> &'static str {
        match self {
            Self::InputEvent { .. } => "input",
            Self::InputCell { .. } => "cell",
            Self::Map { .. } => "map",
            Self::Filter { .. } => "filter",
            Self::Merge => "merge",
            Self::Snapshot { .. } => "snapshot",
            Self::Hold { .. } => "hold",
            Self::Lift { .. } => "lift",
            Self::Never => "never",
            Self::Sink => "output",
            Self::Component { .. } => "component",
        }
    }

    pub fn is_component(&self) -> bool {
        matches!(self, Self::Component { .. })
    }

    pub fn is_event(&self) -> bool {
        matches!(
            self,
            Self::InputEvent { .. }
                | Self::Map { .. }
                | Self::Filter { .. }
                | Self::Merge
                | Self::Snapshot { .. }
                | Self::Never
        )
    }

    pub fn is_cell(&self) -> bool {
        matches!(
            self,
            Self::InputCell { .. } | Self::Hold { .. } | Self::Lift { .. }
        )
    }

    pub fn is_sink(&self) -> bool {
        matches!(self, Self::Sink)
    }

    pub fn is_source(&self) -> bool {
        matches!(self, Self::InputEvent { .. } | Self::InputCell { .. })
    }

    pub fn output_type(&self) -> Option<PortType> {
        self.output_types().into_iter().next()
    }

    pub fn output_types(&self) -> Vec<PortType> {
        match self {
            Self::Component { outputs, .. } => outputs.iter().map(|port| port.ty).collect(),
            _ if self.is_event() => vec![PortType::Stream],
            _ if self.is_cell() => vec![PortType::Cell],
            _ => Vec::new(),
        }
    }

    pub fn input_types(&self) -> Vec<PortType> {
        match self {
            Self::Map { .. } | Self::Filter { .. } | Self::Hold { .. } => {
                vec![PortType::Stream]
            }
            Self::Merge => vec![PortType::Stream, PortType::Stream],
            Self::Snapshot { .. } => vec![PortType::Stream, PortType::Cell],
            Self::Lift { .. } => vec![PortType::Cell, PortType::Cell],
            Self::Sink => vec![PortType::Any],
            Self::Component { inputs, .. } => inputs.iter().map(|port| port.ty).collect(),
            _ => vec![],
        }
    }

    pub fn port_label(&self, index: usize) -> &str {
        match (self, index) {
            (Self::Snapshot { .. }, 0) => "event",
            (Self::Snapshot { .. }, 1) => "cell",
            (Self::Merge, 0) => "left",
            (Self::Merge, 1) => "right",
            (Self::Lift { .. }, 0) => "a",
            (Self::Lift { .. }, 1) => "b",
            (Self::Component { inputs, .. }, index) => inputs
                .get(index)
                .map(|port| port.name.as_str())
                .unwrap_or(""),
            _ => "",
        }
    }

    pub fn output_port_label(&self, index: usize) -> &str {
        match self {
            Self::Component { outputs, .. } => outputs
                .get(index)
                .map(|port| port.name.as_str())
                .unwrap_or(""),
            _ => "",
        }
    }

    pub fn body_text(&self) -> String {
        match self {
            Self::InputEvent { .. } => String::new(),
            Self::InputCell { initial, .. } => format!("= {}", initial.short(14)),
            Self::Map { op } => op.label(),
            Self::Filter { op } => op.label(),
            Self::Merge => String::new(),
            Self::Snapshot { op } | Self::Lift { op } => op.symbol().to_string(),
            Self::Hold { initial } => initial.short(14),
            Self::Never => String::new(),
            Self::Sink => String::new(),
            Self::Component { body, .. } => body.join(" / "),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Node {
    pub id: NodeId,
    pub kind: NodeKind,
    pub name: String,
    #[serde(default)]
    pub label: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub display_only: bool,
    pub pos: [f32; 2],
}

impl Node {
    pub fn new(id: NodeId, kind: NodeKind, pos: [f32; 2]) -> Self {
        let name = kind.primitive().to_string();
        Self {
            id,
            kind,
            name,
            label: String::new(),
            display_only: false,
            pos,
        }
    }

    pub fn size(&self) -> [f32; 2] {
        match &self.kind {
            NodeKind::InputEvent { .. } | NodeKind::InputCell { .. } | NodeKind::Sink => {
                let name = if self.name.is_empty() {
                    self.kind.primitive()
                } else {
                    self.name.as_str()
                };
                let width = (text_width(name, 13.5) + 18.0).clamp(36.0, 280.0);
                [width, 24.0]
            }
            NodeKind::Merge => [150.0, 88.0],
            NodeKind::Component {
                inputs,
                outputs,
                body,
                ..
            } => {
                let name_width = text_width(&self.name, 15.0);
                let left_width = inputs
                    .iter()
                    .map(|port| text_width(&port.name, 10.5))
                    .fold(0.0f32, f32::max);
                let right_width = outputs
                    .iter()
                    .map(|port| text_width(&port.name, 10.5))
                    .fold(0.0f32, f32::max);
                let body_width = body
                    .iter()
                    .map(|line| text_width(line, 10.5))
                    .fold(0.0f32, f32::max);
                let width = (left_width + right_width + name_width + 56.0)
                    .max(body_width + 34.0)
                    .clamp(220.0, 620.0);
                let rows = inputs.len().max(outputs.len()).max(body.len());
                let height = (86.0 + rows as f32 * 18.0).max(108.0);
                [width, height]
            }
            _ if !self.label.is_empty() => {
                let width = (text_width(&self.label, 14.0) + 50.0).clamp(170.0, 380.0);
                [width, 74.0]
            }
            _ => [150.0, 74.0],
        }
    }

    pub fn body_text(&self) -> String {
        if self.label.is_empty() {
            self.kind.body_text()
        } else {
            self.label.clone()
        }
    }

    pub fn rect(&self) -> Rect {
        let [w, h] = self.size();
        Rect::new(self.pos[0], self.pos[1], w, h)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Edge {
    pub id: u64,
    pub from: NodeId,
    pub to: NodeId,
    #[serde(default)]
    pub from_port: usize,
    #[serde(default)]
    pub to_port: usize,
    #[serde(default)]
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_width: Option<f32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Document {
    #[serde(default = "default_title")]
    pub title: String,
    #[serde(default)]
    pub nodes: Vec<Node>,
    #[serde(default)]
    pub edges: Vec<Edge>,
    #[serde(default)]
    pub components: Vec<ComponentDef>,
    #[serde(default)]
    pub annotations: Vec<Annotation>,
    #[serde(default, skip_serializing_if = "is_executable")]
    pub mode: DocumentMode,
    #[serde(default = "default_time_end")]
    pub time_end: f64,
}

fn default_title() -> String {
    "FRP diagram".into()
}

fn default_time_end() -> f64 {
    6.0
}

impl Default for Document {
    fn default() -> Self {
        Self::blank()
    }
}

impl Document {
    pub fn blank() -> Self {
        Self {
            title: "FRP diagram".into(),
            nodes: Vec::new(),
            edges: Vec::new(),
            components: Vec::new(),
            annotations: Vec::new(),
            mode: DocumentMode::Executable,
            time_end: 6.0,
        }
    }

    pub fn next_node_id(&self) -> NodeId {
        self.nodes.iter().map(|n| n.id).max().unwrap_or(0) + 1
    }

    pub fn next_edge_id(&self) -> u64 {
        self.edges.iter().map(|e| e.id).max().unwrap_or(0) + 1
    }

    pub fn add_node(&mut self, kind: NodeKind, pos: [f32; 2]) -> NodeId {
        let id = self.next_node_id();
        self.nodes.push(Node::new(id, kind, pos));
        id
    }

    pub fn remove_node(&mut self, id: NodeId) {
        self.nodes.retain(|n| n.id != id);
        self.edges.retain(|e| e.from != id && e.to != id);
    }

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }

    pub fn node_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.iter_mut().find(|n| n.id == id)
    }

    pub fn edge(&self, id: u64) -> Option<&Edge> {
        self.edges.iter().find(|e| e.id == id)
    }

    pub fn incoming_edge(&self, node: NodeId, port: usize) -> Option<&Edge> {
        self.edges
            .iter()
            .find(|e| e.to == node && e.to_port == port)
    }

    pub fn incoming_edges(&self, node: NodeId) -> Vec<&Edge> {
        self.edges.iter().filter(|e| e.to == node).collect()
    }

    pub fn outgoing_edges(&self, node: NodeId) -> Vec<&Edge> {
        self.edges.iter().filter(|e| e.from == node).collect()
    }

    pub fn validate_edge(&self, from: NodeId, to: NodeId, to_port: usize) -> Result<(), String> {
        self.validate_edge_from(from, 0, to, to_port)
    }

    pub fn validate_edge_from(
        &self,
        from: NodeId,
        from_port: usize,
        to: NodeId,
        to_port: usize,
    ) -> Result<(), String> {
        if from == to {
            return Err("A node cannot be connected to itself".into());
        }
        let source = self.node(from).ok_or("Unknown source node")?;
        let target = self.node(to).ok_or("Unknown target node")?;
        let source_type = *source
            .kind
            .output_types()
            .get(from_port)
            .ok_or("The source node has no such output port")?;
        let input_types = target.kind.input_types();
        let target_type = *input_types
            .get(to_port)
            .ok_or("The target node has no such input port")?;
        if !target_type.accepts(source_type) {
            return Err(format!(
                "{} output cannot connect to {} input",
                source_type.name(),
                target_type.name()
            ));
        }
        if let Some(existing) = self.incoming_edge(to, to_port) {
            return Err(format!(
                "Input port already connected from '{}'",
                self.node(existing.from)
                    .map(|n| n.name.as_str())
                    .unwrap_or("?")
            ));
        }
        Ok(())
    }

    pub fn add_edge(&mut self, from: NodeId, to: NodeId, to_port: usize) -> Result<u64, String> {
        self.add_edge_from_port(from, 0, to, to_port)
    }

    pub fn add_edge_from_port(
        &mut self,
        from: NodeId,
        from_port: usize,
        to: NodeId,
        to_port: usize,
    ) -> Result<u64, String> {
        self.validate_edge_from(from, from_port, to, to_port)?;
        let id = self.next_edge_id();
        self.edges.push(Edge {
            id,
            from,
            from_port,
            to,
            to_port,
            label: String::new(),
            label_width: None,
        });
        Ok(id)
    }

    pub fn remove_edge(&mut self, id: u64) {
        self.edges.retain(|e| e.id != id);
    }

    pub fn all_event_times(&self) -> Vec<f64> {
        let mut times: Vec<f64> = self
            .nodes
            .iter()
            .flat_map(|n| match &n.kind {
                NodeKind::InputEvent { events } => {
                    events.iter().map(|e| e.time).collect::<Vec<_>>()
                }
                NodeKind::InputCell { changes, .. } => {
                    changes.iter().map(|e| e.time).collect::<Vec<_>>()
                }
                _ => Vec::new(),
            })
            .filter(|t| t.is_finite() && *t >= 0.0)
            .collect();
        times.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        times.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
        times
    }

    pub fn latest_event_time(&self) -> f64 {
        self.all_event_times().last().copied().unwrap_or(0.0)
    }

    pub fn effective_end_time(&self) -> f64 {
        self.time_end.max(self.latest_event_time() + 0.8).max(1.0)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn center(&self) -> [f32; 2] {
        [self.x + self.w / 2.0, self.y + self.h / 2.0]
    }

    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    pub fn contains(&self, p: [f32; 2]) -> bool {
        p[0] >= self.x && p[0] <= self.right() && p[1] >= self.y && p[1] <= self.bottom()
    }

    pub fn expand(&self, amount: f32) -> Self {
        Self::new(
            self.x - amount,
            self.y - amount,
            self.w + amount * 2.0,
            self.h + amount * 2.0,
        )
    }
}

pub fn text_width(text: &str, size: f32) -> f32 {
    text.chars()
        .map(|c| {
            if c.is_ascii() {
                size * 0.56
            } else if c.is_whitespace() {
                size * 0.3
            } else {
                size * 0.98
            }
        })
        .sum()
}

pub fn fit_text(text: &str, max_width: f32, size: f32) -> String {
    if text_width(text, size) <= max_width {
        return text.to_string();
    }
    let mut out = String::new();
    let mut width = 0.0;
    let ellipsis_width = size * 0.9;
    for c in text.chars() {
        let w = if c.is_ascii() {
            size * 0.56
        } else {
            size * 0.98
        };
        if width + w + ellipsis_width > max_width {
            out.push('…');
            return out;
        }
        out.push(c);
        width += w;
    }
    out
}
