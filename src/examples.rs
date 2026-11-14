use crate::model::{
    Annotation, AnnotationTarget, CombineOp, ComponentDef, Document, FilterOp, NodeId, NodeKind,
    PortSpec, PortType, TimedValue, UnaryOp, Value,
};

pub const EXAMPLES: &[(&str, &str)] = &[
    ("Accumulator", "accumulator"),
    ("Map & filter", "map_filter"),
    ("Snapshot", "snapshot"),
    ("Merge", "merge"),
    ("OrElse", "or_else"),
    ("Lift cells", "lift"),
    ("Calibration", "calibration"),
    ("Component", "component"),
    ("Annotations", "annotation"),
];

pub fn load_example(name: &str) -> Option<Document> {
    match name {
        "accumulator" | "Accumulator" => Some(accumulator()),
        "map_filter" | "Map & filter" => Some(map_filter()),
        "snapshot" | "Snapshot" => Some(snapshot()),
        "merge" | "Merge" => Some(merge()),
        "or_else" | "OrElse" | "orElse" => Some(or_else()),
        "lift" | "Lift cells" => Some(lift()),
        "calibration" | "Calibration" => Some(calibration()),
        "component" | "Component" => Some(component()),
        "annotation" | "Annotations" => Some(annotation()),
        _ => None,
    }
}

fn label(doc: &mut Document, from: NodeId, to: NodeId, port: usize, text: &str) {
    if let Some(edge) = doc
        .edges
        .iter_mut()
        .find(|e| e.from == from && e.to == to && e.to_port == port)
    {
        edge.label = text.to_string();
    }
}

fn add(doc: &mut Document, kind: NodeKind, name: &str, _note: &str, pos: [f32; 2]) -> NodeId {
    let id = doc.add_node(kind, pos);
    if let Some(node) = doc.node_mut(id) {
        node.name = name.to_string();
    }
    id
}

fn accumulator() -> Document {
    let mut doc = Document::blank();
    doc.title = "Accumulator — hold · snapshot · merge".into();
    let delta = add(
        &mut doc,
        NodeKind::InputEvent {
            events: vec![
                TimedValue::new(1.0, Value::Number(5.0)),
                TimedValue::new(2.0, Value::Number(10.0)),
                TimedValue::new(3.0, Value::Number(-3.0)),
                TimedValue::new(4.0, Value::Number(8.0)),
            ],
        },
        "sDelta",
        "input stream",
        [40.0, 120.0],
    );
    let total = add(
        &mut doc,
        NodeKind::Snapshot {
            op: CombineOp::Add,
            cells: 1,
        },
        "snapshot",
        "",
        [300.0, 120.0],
    );
    let hold = add(
        &mut doc,
        NodeKind::Hold {
            initial: Value::Number(0.0),
        },
        "accum",
        "",
        [300.0, 300.0],
    );
    let sink = add(&mut doc, NodeKind::Sink, "total", "SLabel", [560.0, 120.0]);
    doc.add_edge(delta, total, 0).unwrap();
    doc.add_edge(hold, total, 1).unwrap();
    doc.add_edge(total, hold, 0).unwrap();
    doc.add_edge(total, sink, 0).unwrap();
    label(&mut doc, delta, total, 0, "delta");
    doc.time_end = 5.0;
    doc
}

fn map_filter() -> Document {
    let mut doc = Document::blank();
    doc.title = "Map & filter".into();
    let input = add(
        &mut doc,
        NodeKind::InputEvent {
            events: vec![
                TimedValue::new(1.0, Value::Number(-2.0)),
                TimedValue::new(2.0, Value::Number(0.0)),
                TimedValue::new(3.0, Value::Number(3.0)),
                TimedValue::new(4.0, Value::Number(5.0)),
            ],
        },
        "sNumber",
        "input stream",
        [40.0, 90.0],
    );
    let map = add(
        &mut doc,
        NodeKind::Map {
            op: UnaryOp::Add(1.0),
            cell: false,
        },
        "map",
        "",
        [280.0, 90.0],
    );
    let filter = add(
        &mut doc,
        NodeKind::Filter {
            op: FilterOp::Positive,
        },
        "filter",
        "",
        [520.0, 90.0],
    );
    let sink = add(&mut doc, NodeKind::Sink, "out", "output", [760.0, 84.0]);
    doc.add_edge(input, map, 0).unwrap();
    doc.add_edge(map, filter, 0).unwrap();
    doc.add_edge(filter, sink, 0).unwrap();
    label(&mut doc, input, map, 0, "x");
    doc.time_end = 5.0;
    doc
}

fn snapshot() -> Document {
    let mut doc = Document::blank();
    doc.title = "Snapshot — sampling a cell".into();
    let clicked = add(
        &mut doc,
        NodeKind::InputEvent {
            events: vec![
                TimedValue::new(1.0, Value::Unit),
                TimedValue::new(2.0, Value::Unit),
                TimedValue::new(3.0, Value::Unit),
            ],
        },
        "sClicked",
        "SButton",
        [40.0, 40.0],
    );
    let text = add(
        &mut doc,
        NodeKind::InputCell {
            initial: Value::Text("I like FRP".into()),
            changes: vec![TimedValue::new(2.0, Value::Text("FRP is fun".into()))],
        },
        "text",
        "",
        [40.0, 220.0],
    );
    let snap = add(
        &mut doc,
        NodeKind::Snapshot {
            op: CombineOp::KeepRight,
            cells: 1,
        },
        "snapshot",
        "",
        [310.0, 130.0],
    );
    let map = add(
        &mut doc,
        NodeKind::Map {
            op: UnaryOp::Reverse,
            cell: false,
        },
        "map",
        "",
        [530.0, 130.0],
    );
    let sink = add(&mut doc, NodeKind::Sink, "sLatin", "SLabel", [750.0, 124.0]);
    doc.add_edge(clicked, snap, 0).unwrap();
    doc.add_edge(text, snap, 1).unwrap();
    doc.add_edge(snap, map, 0).unwrap();
    doc.add_edge(map, sink, 0).unwrap();
    label(&mut doc, clicked, snap, 0, "clicked");
    label(&mut doc, text, snap, 1, "text");
    doc.time_end = 4.0;
    doc
}

fn merge() -> Document {
    let mut doc = Document::blank();
    doc.title = "Merge — interleaving two streams".into();
    let left = add(
        &mut doc,
        NodeKind::InputEvent {
            events: vec![
                TimedValue::new(1.0, Value::Number(1.0)),
                TimedValue::new(3.0, Value::Number(3.0)),
                TimedValue::new(5.0, Value::Number(5.0)),
            ],
        },
        "sLeft",
        "left input",
        [40.0, 40.0],
    );
    let right = add(
        &mut doc,
        NodeKind::InputEvent {
            events: vec![
                TimedValue::new(2.0, Value::Number(2.0)),
                TimedValue::new(4.0, Value::Number(4.0)),
                TimedValue::new(6.0, Value::Number(6.0)),
            ],
        },
        "sRight",
        "right input",
        [40.0, 220.0],
    );
    let merge = add(&mut doc, NodeKind::Merge, "merge", "", [310.0, 130.0]);
    let sink = add(
        &mut doc,
        NodeKind::Sink,
        "sChange",
        "output",
        [560.0, 126.0],
    );
    doc.add_edge(left, merge, 0).unwrap();
    doc.add_edge(right, merge, 1).unwrap();
    doc.add_edge(merge, sink, 0).unwrap();
    doc.time_end = 7.0;
    doc
}

fn or_else() -> Document {
    let mut doc = Document::blank();
    doc.title = "OrElse — left-biased merge".into();
    let left = add(
        &mut doc,
        NodeKind::InputEvent {
            events: vec![
                TimedValue::new(1.0, Value::Number(1.0)),
                TimedValue::new(3.0, Value::Number(3.0)),
                TimedValue::new(5.0, Value::Number(5.0)),
            ],
        },
        "sLeft",
        "left input",
        [40.0, 40.0],
    );
    let right = add(
        &mut doc,
        NodeKind::InputEvent {
            events: vec![
                TimedValue::new(2.0, Value::Number(20.0)),
                TimedValue::new(3.0, Value::Number(30.0)),
                TimedValue::new(4.0, Value::Number(40.0)),
            ],
        },
        "sRight",
        "right input",
        [40.0, 220.0],
    );
    let or_else = add(&mut doc, NodeKind::OrElse, "orElse", "", [310.0, 130.0]);
    let sink = add(
        &mut doc,
        NodeKind::Sink,
        "sChange",
        "output",
        [560.0, 126.0],
    );
    doc.add_edge(left, or_else, 0).unwrap();
    doc.add_edge(right, or_else, 1).unwrap();
    doc.add_edge(or_else, sink, 0).unwrap();
    doc.time_end = 6.0;
    doc
}

fn lift() -> Document {
    let mut doc = Document::blank();
    doc.title = "Lift — combining cells".into();
    let a = add(
        &mut doc,
        NodeKind::InputCell {
            initial: Value::Number(5.0),
            changes: vec![TimedValue::new(2.0, Value::Number(7.0))],
        },
        "a",
        "",
        [40.0, 40.0],
    );
    let b = add(
        &mut doc,
        NodeKind::InputCell {
            initial: Value::Number(10.0),
            changes: vec![TimedValue::new(3.0, Value::Number(4.0))],
        },
        "b",
        "",
        [40.0, 220.0],
    );
    let lift = add(
        &mut doc,
        NodeKind::Lift { op: CombineOp::Add },
        "lift",
        "",
        [310.0, 130.0],
    );
    let sink = add(&mut doc, NodeKind::Sink, "sum", "output", [560.0, 126.0]);
    doc.add_edge(a, lift, 0).unwrap();
    doc.add_edge(b, lift, 1).unwrap();
    doc.add_edge(lift, sink, 0).unwrap();
    doc.time_end = 4.0;
    doc
}

fn calibration() -> Document {
    let mut doc = Document::blank();
    doc.title = "Calibration — clearing and scaling an accumulator".into();
    let clear = add(
        &mut doc,
        NodeKind::InputEvent {
            events: vec![TimedValue::new(3.0, Value::Unit)],
        },
        "sClearAccumulator",
        "input stream",
        [20.0, 30.0],
    );
    let clear_zero = add(
        &mut doc,
        NodeKind::Map {
            op: UnaryOp::Constant(0.0),
            cell: false,
        },
        "map",
        "",
        [250.0, 20.0],
    );
    let delta = add(
        &mut doc,
        NodeKind::InputEvent {
            events: vec![
                TimedValue::new(1.0, Value::Number(5.0)),
                TimedValue::new(2.0, Value::Number(10.0)),
                TimedValue::new(4.0, Value::Number(-3.0)),
                TimedValue::new(5.0, Value::Number(8.0)),
            ],
        },
        "sDelta",
        "input stream",
        [20.0, 180.0],
    );
    let snap = add(
        &mut doc,
        NodeKind::Snapshot {
            op: CombineOp::Add,
            cells: 1,
        },
        "snapshot",
        "",
        [470.0, 150.0],
    );
    let hold = add(
        &mut doc,
        NodeKind::Hold {
            initial: Value::Number(0.0),
        },
        "hold",
        "",
        [470.0, 360.0],
    );
    let merge = add(&mut doc, NodeKind::Merge, "merge", "", [700.0, 130.0]);
    let calibration = add(
        &mut doc,
        NodeKind::InputCell {
            initial: Value::Number(1.0),
            changes: vec![TimedValue::new(4.0, Value::Number(2.0))],
        },
        "calibration",
        "",
        [700.0, 420.0],
    );
    let lift = add(
        &mut doc,
        NodeKind::Lift {
            op: CombineOp::Multiply,
        },
        "lift",
        "",
        [950.0, 320.0],
    );
    let sink = add(&mut doc, NodeKind::Sink, "total", "SLabel", [1170.0, 314.0]);

    doc.add_edge(clear, clear_zero, 0).unwrap();
    doc.add_edge(delta, snap, 0).unwrap();
    doc.add_edge(hold, snap, 1).unwrap();
    doc.add_edge(clear_zero, merge, 0).unwrap();
    doc.add_edge(snap, merge, 1).unwrap();
    doc.add_edge(merge, hold, 0).unwrap();
    doc.add_edge(hold, lift, 0).unwrap();
    doc.add_edge(calibration, lift, 1).unwrap();
    doc.add_edge(lift, sink, 0).unwrap();
    label(&mut doc, delta, snap, 0, "delta");
    label(&mut doc, clear_zero, merge, 0, "clear");
    label(&mut doc, snap, merge, 1, "newTotal");
    doc.time_end = 6.0;
    doc
}

fn component() -> Document {
    let inputs = vec![
        PortSpec {
            name: "delta".into(),
            ty: PortType::Stream,
        },
        PortSpec {
            name: "clear".into(),
            ty: PortType::Stream,
        },
    ];
    let outputs = vec![
        PortSpec {
            name: "total".into(),
            ty: PortType::Stream,
        },
        PortSpec {
            name: "state".into(),
            ty: PortType::Cell,
        },
    ];

    let mut internal = Document::blank();
    internal.title = "accumulate".into();
    let delta = add(
        &mut internal,
        NodeKind::InputEvent { events: Vec::new() },
        "delta",
        "",
        [40.0, 80.0],
    );
    let clear = add(
        &mut internal,
        NodeKind::InputEvent { events: Vec::new() },
        "clear",
        "",
        [40.0, 260.0],
    );
    let clear_zero = add(
        &mut internal,
        NodeKind::Map {
            op: UnaryOp::Constant(0.0),
            cell: false,
        },
        "clearZero",
        "",
        [260.0, 250.0],
    );
    let snapshot = add(
        &mut internal,
        NodeKind::Snapshot {
            op: CombineOp::Add,
            cells: 1,
        },
        "snapshot",
        "",
        [500.0, 80.0],
    );
    let merge = add(&mut internal, NodeKind::Merge, "merge", "", [720.0, 60.0]);
    let hold = add(
        &mut internal,
        NodeKind::Hold {
            initial: Value::Number(0.0),
        },
        "state",
        "",
        [720.0, 280.0],
    );
    let total = add(&mut internal, NodeKind::Sink, "total", "", [980.0, 60.0]);
    let state = add(&mut internal, NodeKind::Sink, "state", "", [980.0, 300.0]);

    internal.add_edge(clear, clear_zero, 0).unwrap();
    internal.add_edge(clear_zero, merge, 0).unwrap();
    internal.add_edge(delta, snapshot, 0).unwrap();
    internal.add_edge(hold, snapshot, 1).unwrap();
    internal.add_edge(snapshot, merge, 1).unwrap();
    internal.add_edge(merge, hold, 0).unwrap();
    internal.add_edge(merge, total, 0).unwrap();
    internal.add_edge(hold, state, 0).unwrap();
    internal.time_end = 6.0;

    let mut doc = Document::blank();
    doc.title = "Component — accumulate cloud".into();
    doc.components.push(ComponentDef {
        name: "accumulate".into(),
        inputs: inputs.clone(),
        outputs: outputs.clone(),
        body: vec!["hold · snapshot".into()],
        doc: Box::new(internal),
    });

    let s_delta = add(
        &mut doc,
        NodeKind::InputEvent {
            events: vec![
                TimedValue::new(1.0, Value::Number(5.0)),
                TimedValue::new(2.0, Value::Number(10.0)),
                TimedValue::new(4.0, Value::Number(-3.0)),
                TimedValue::new(5.0, Value::Number(8.0)),
            ],
        },
        "sDelta",
        "input stream",
        [40.0, 80.0],
    );
    let s_clear = add(
        &mut doc,
        NodeKind::InputEvent {
            events: vec![TimedValue::new(3.0, Value::Unit)],
        },
        "sClear",
        "input stream",
        [40.0, 260.0],
    );
    let accumulated = add(
        &mut doc,
        NodeKind::Component {
            definition: Some("accumulate".into()),
            inputs,
            outputs,
            body: vec!["hold · snapshot".into()],
        },
        "accumulate",
        "",
        [320.0, 110.0],
    );
    let observed = add(
        &mut doc,
        NodeKind::Map {
            op: UnaryOp::Identity,
            cell: false,
        },
        "observed",
        "",
        [650.0, 140.0],
    );
    let sink = add(&mut doc, NodeKind::Sink, "total", "SLabel", [880.0, 134.0]);
    let state_sink = add(&mut doc, NodeKind::Sink, "state", "SLabel", [880.0, 300.0]);

    doc.add_edge_from_port(s_delta, 0, accumulated, 0).unwrap();
    doc.add_edge_from_port(s_clear, 0, accumulated, 1).unwrap();
    doc.add_edge_from_port(accumulated, 0, observed, 0).unwrap();
    doc.add_edge_from_port(accumulated, 1, state_sink, 0)
        .unwrap();
    doc.add_edge(observed, sink, 0).unwrap();
    doc.time_end = 6.0;
    doc
}

fn annotation() -> Document {
    let mut doc = Document::blank();
    doc.title = "Annotations — network callout".into();
    let source = add(
        &mut doc,
        NodeKind::InputEvent {
            events: vec![
                TimedValue::new(1.0, Value::Number(1.0)),
                TimedValue::new(2.0, Value::Number(2.0)),
                TimedValue::new(3.0, Value::Number(3.0)),
            ],
        },
        "sNumber",
        "",
        [40.0, 80.0],
    );
    let increment = add(
        &mut doc,
        NodeKind::Map {
            op: UnaryOp::Add(1.0),
            cell: false,
        },
        "increment",
        "",
        [300.0, 80.0],
    );
    let sink = add(&mut doc, NodeKind::Sink, "out", "", [600.0, 74.0]);
    doc.add_edge(source, increment, 0).unwrap();
    doc.add_edge(increment, sink, 0).unwrap();
    doc.annotations.push(Annotation {
        text: "each event is incremented by one".into(),
        target: Some(AnnotationTarget::Node { id: increment }),
        position: Some([300.0, 260.0]),
    });
    doc.time_end = 4.0;
    doc
}
