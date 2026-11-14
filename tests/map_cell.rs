use frp_visualizer::{
    dsl, execution,
    model::{Document, NodeKind, Value},
    render_svgs, sim,
};

#[test]
fn map_on_cell_initializes_updates_and_snapshot_reads_the_old_value() {
    let doc = dsl::parse(include_str!("../examples/map-cell.frp"))
        .unwrap()
        .doc;
    let id = |name| doc.nodes.iter().find(|node| node.name == name).unwrap().id;
    let simulation = sim::simulate(&doc);
    let states: Vec<_> = simulation
        .cell(id("doubled"))
        .iter()
        .map(|step| (step.time, step.value.to_string()))
        .collect();
    assert_eq!(
        states,
        vec![(0.0, "4".into()), (1.0, "6".into()), (2.0, "8".into())]
    );
    let observed: Vec<_> = simulation
        .stream(id("observed"))
        .iter()
        .map(|event| event.value.to_string())
        .collect();
    assert_eq!(observed, ["4", "6", "8"]);
    assert!(sim::validate_document(&doc).errors.is_empty());
    let restored: Document = serde_json::from_str(&serde_json::to_string(&doc).unwrap()).unwrap();
    assert!(matches!(
        restored.node(id("doubled")).unwrap().kind,
        NodeKind::Map { cell: true, .. }
    ));
    let model = execution::build_execution_model(&restored);
    let lane = model
        .lanes
        .iter()
        .find(|lane| lane.source == id("doubled"))
        .unwrap();
    assert_eq!(model.cell(lane).len(), 3);
    let svg = render_svgs(&restored);
    assert!(svg.network.contains(">map</text>"));
    assert!(svg.timing.contains("doubled"));
}

#[test]
fn chained_maps_on_cells_observe_upstream_updates_in_dependency_order() {
    let doc = dsl::parse("map second = add 1\nmap first = mul 2\ncell input = 3 changes 1:5\ninput -> first.in\nfirst.out -> second.in\n").unwrap().doc;
    let id = doc
        .nodes
        .iter()
        .find(|node| node.name == "second")
        .unwrap()
        .id;
    let values: Vec<_> = sim::simulate(&doc)
        .cell(id)
        .iter()
        .map(|step| step.value.to_string())
        .collect();
    assert_eq!(values, ["7", "11"]);
}

#[test]
fn map_on_cell_does_not_emit_a_stream_or_duplicate_unchanged_values() {
    let doc = dsl::parse(
        "cell input = false changes 1:true, 2:true\nmap inverse = not\ninput -> inverse\n",
    )
    .unwrap()
    .doc;
    let id = doc
        .nodes
        .iter()
        .find(|node| node.name == "inverse")
        .unwrap()
        .id;
    let simulation = sim::simulate(&doc);
    assert!(simulation.stream(id).is_empty());
    assert_eq!(simulation.cell(id).len(), 2);
    assert_eq!(simulation.cell(id)[0].value, Value::Bool(true));
    assert_eq!(simulation.cell(id)[1].value, Value::Bool(false));
}

#[test]
fn map_on_cell_expands_inside_a_component() {
    let doc = dsl::parse("component double {\n in input: cell\n out result: cell\n map m = mul 2\n input -> m\n m -> result\n}\ncell input = 3 changes 1:5\ncomponent c = double\ninput -> c.input\nsink out\nc.result -> out\n").unwrap().doc;
    assert!(sim::validate_document(&doc).errors.is_empty());
    let model = execution::build_execution_model(&doc);
    let lane = model.lanes.iter().find(|lane| lane.label == "out").unwrap();
    let values: Vec<_> = model
        .cell(lane)
        .iter()
        .map(|step| step.value.to_string())
        .collect();
    assert_eq!(values, ["6", "10"]);
}

#[test]
fn map_on_cell_checks_types_and_display_only_execution() {
    assert!(dsl::parse("stream input events 1:1\nmap mapped = add 1\nsnapshot s = add\ninput -> mapped\nmapped -> s.cell\n").is_err());
    assert!(dsl::parse(
        "cell input = 1\nmap mapped = add 1\nhold state = 0\ninput -> mapped\nmapped -> state\n"
    )
    .is_err());
    let doc = dsl::parse("map mapped = add 1\n").unwrap().doc;
    assert!(!sim::validate_document(&doc).errors.is_empty());
    let source = "cell input = 1\nmap mapped = display \"構造体から値を取得\"\ninput -> mapped\n";
    let doc = dsl::parse(source).unwrap().doc;
    assert!(!sim::validate_document(&doc).errors.is_empty());
    let doc = dsl::parse(&format!("mode structural\n{source}"))
        .unwrap()
        .doc;
    assert!(sim::validate_document(&doc).errors.is_empty());
}

#[test]
fn map_infers_cell_chains_even_when_wires_are_written_backwards() {
    let doc = dsl::parse("cell input = 3 changes 1:5\nmap first = mul 2\nmap second = add 1\nstream tick events 1:(), 2:()\nsnapshot s = keep_right\nsecond -> s.cell\nfirst -> second\ninput -> first\ntick -> s.event\n").unwrap().doc;
    let id = doc.nodes.iter().find(|node| node.name == "s").unwrap().id;
    let simulation = sim::simulate(&doc);
    let values: Vec<_> = simulation
        .stream(id)
        .iter()
        .map(|event| event.value.to_string())
        .collect();
    assert_eq!(values, ["7", "11"]);
}

#[test]
fn map_infers_component_output_ports_and_internal_chains() {
    let doc = dsl::parse("component mixed {\n in event: stream\n in state: cell\n out stream: stream\n out cell: cell\n map first = mul 2\n map second = add 1\n first -> second\n state -> first\n event -> stream\n second -> cell\n}\nstream event events 1:42\ncell state = 3 changes 1:5\ncomponent c = mixed\nmap mapped = mul 10\nsink out\nmapped -> out\nc.cell -> mapped\nevent -> c.event\nstate -> c.state\n").unwrap().doc;
    assert!(sim::validate_document(&doc).errors.is_empty());
    let model = execution::build_execution_model(&doc);
    let lane = model.lanes.iter().find(|lane| lane.label == "out").unwrap();
    let values: Vec<_> = model
        .cell(lane)
        .iter()
        .map(|step| step.value.to_string())
        .collect();
    assert_eq!(values, ["70", "110"]);
}

#[test]
fn one_map_primitive_uses_color_to_distinguish_stream_and_cell() {
    use frp_visualizer::{
        layout, render_network,
        scene::{Shape, BLUE, YELLOW},
    };
    let doc = dsl::parse("stream event events 1:2\ncell state = 3\nmap sm = add 1\nmap cm = add 1\nevent -> sm\nstate -> cm\n").unwrap().doc;
    let network = layout::layout(&doc);
    let scene = render_network::render_network(&doc, &network);
    for (name, expected) in [("sm", YELLOW), ("cm", BLUE)] {
        let node = doc.nodes.iter().find(|node| node.name == name).unwrap();
        assert_eq!(node.kind.primitive(), "map");
        let rect = network.node_rects[&node.id];
        assert!(scene.shapes.iter().any(|shape| matches!(shape,
            Shape::Path { points, fill, .. } if points.len() == 5 && points[0] == [rect.x, rect.y] && *fill == expected)));
    }
    let legacy: NodeKind =
        serde_json::from_str(r#"{"type":"map","op":{"op":"identity"}}"#).unwrap();
    assert!(legacy.is_event());
    assert!(serde_json::to_value(legacy).unwrap().get("cell").is_none());
    let cell_map = doc.nodes.iter().find(|node| node.name == "cm").unwrap();
    let json = serde_json::to_value(&cell_map.kind).unwrap();
    assert_eq!(json["type"], "map");
    assert_eq!(json["cell"], true);
}

#[test]
fn deferred_map_inference_still_rejects_invalid_wires() {
    let prefix =
        "stream event events 1:1\ncell state = 2\nmap m = add 1\nhold h = 0\nsnapshot s = add\n";
    for wires in [
        "m -> h\nstate -> m\n",
        "m -> s.cell\nevent -> m\n",
        "event -> m\nstate -> m\n",
        "state -> m.1\n",
        "m -> m\n",
    ] {
        assert!(dsl::parse(&format!("{prefix}{wires}")).is_err(), "{wires}");
    }
    assert!(dsl::parse("cellmap m = add 1\n").is_err());
}

#[test]
fn map_diagnostics_preserve_existing_wording_and_wire_line_numbers() {
    for (source, expected) in [
        ("map m\n", "line 1: expected 'map <name> = <op>'"),
        (
            "map m = display \"処理\" extra\n",
            "line 1: unexpected text after display label",
        ),
        (
            "stream input\nmap m = identity\nsnapshot s = add\ninput -> m\nm -> s.cell\n",
            "line 5: stream output cannot connect to cell input",
        ),
        (
            "cell input = 1\nmap m = identity\nhold h = 0\nm -> h\ninput -> m\n",
            "line 4: cell output cannot connect to stream input",
        ),
        (
            "component c {\n in input: stream\n map m = identity\n snapshot s = add\n input -> m\n m -> s.cell\n}\n",
            "line 6: stream output cannot connect to cell input",
        ),
        (
            "stream input\nmap m = identity\nsnapshot s = add\nm -> s.cell\ncomponent c {\n in input: stream\n out result: stream\n input -> result\n}\ninput -> m\n",
            "line 4: stream output cannot connect to cell input",
        ),
    ] {
        assert_eq!(dsl::parse(source).unwrap_err(), expected, "{source}");
    }
}

#[test]
fn map_on_cell_example_passes_strict_cli_validation() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_frp-visualizer"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args(["check", "examples/map-cell.frp", "--strict"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
