use frp_visualizer::{
    dsl, execution, layout,
    model::{Document, NodeKind},
    render_svgs, sim,
};

#[test]
fn snapshot_reads_all_old_cells_in_port_order() {
    let doc = dsl::parse(include_str!("../examples/snapshot-multi.frp"))
        .unwrap()
        .doc;
    let node = doc.nodes.iter().find(|node| node.name == "total").unwrap();
    let simulation = sim::simulate(&doc);
    let values: Vec<_> = simulation
        .stream(node.id)
        .iter()
        .map(|event| (event.time, event.value.to_string()))
        .collect();
    assert_eq!(
        values,
        vec![(1.0, "83".into()), (2.0, "71".into()), (3.0, "70".into())]
    );
    assert!(sim::validate_document(&doc).errors.is_empty());
    let restored: Document = serde_json::from_str(&serde_json::to_string(&doc).unwrap()).unwrap();
    assert!(matches!(
        restored.node(node.id).unwrap().kind,
        NodeKind::Snapshot { cells: 3, .. }
    ));
    let svg = render_svgs(&restored);
    for label in ["c1", "c2", "c3"] {
        assert!(svg.network.contains(&format!(">{label}</text>")));
    }
    assert!(svg.timing.contains("total"));

    let rect = node.rect();
    let points: Vec<_> = (1..=3)
        .map(|port| layout::input_port_pos(node, rect, port))
        .collect();
    assert!(points.windows(2).all(|pair| pair[0][0] < pair[1][0]));
    assert!(points
        .iter()
        .all(|point| point[1] == rect.bottom() && point[0] > rect.x && point[0] < rect.right()));
    assert_eq!(
        layout::input_port_pos(node, rect, 0),
        [rect.x, rect.center()[1]]
    );
}

#[test]
fn legacy_snapshot_json_and_cell_alias_still_work() {
    let kind: NodeKind =
        serde_json::from_str(r#"{"type":"snapshot","op":{"op":"keep_right"}}"#).unwrap();
    assert!(matches!(kind, NodeKind::Snapshot { cells: 1, .. }));
    let json = serde_json::to_value(&kind).unwrap();
    assert!(json.get("cells").is_none());
    let doc = dsl::parse("stream input events 1:()\ncell state = 7\nsnapshot observed = keep_right\ninput -> observed.event\nstate -> observed.cell\n").unwrap().doc;
    let id = doc
        .nodes
        .iter()
        .find(|node| node.name == "observed")
        .unwrap()
        .id;
    assert_eq!(sim::simulate(&doc).stream(id)[0].value.to_string(), "7");
}

#[test]
fn snapshot_supports_feedback_and_component_expansion() {
    let doc = dsl::parse("component accumulator {\n in input: stream\n in offset: cell\n out result: stream\n snapshot sum cells 2 = add\n hold state = 0\n input -> sum.event\n state -> sum.c1\n offset -> sum.c2\n sum -> state\n sum -> result\n}\nstream input events 1:1, 2:1, 3:1\ncell offset = 1\ncomponent c = accumulator\ninput -> c.input\noffset -> c.offset\nsink out\nc.result -> out\n").unwrap().doc;
    let report = sim::validate_document(&doc);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let model = execution::build_execution_model(&doc);
    let output = model.lanes.iter().find(|lane| lane.label == "out").unwrap();
    let values: Vec<_> = model
        .stream(output)
        .iter()
        .map(|event| event.value.to_string())
        .collect();
    assert_eq!(values, ["2", "4", "6"]);
}

#[test]
fn snapshot_validates_cell_count_and_port_range() {
    for count in ["0", "256", "-1", "invalid"] {
        assert_eq!(
            dsl::parse(&format!("snapshot s cells {count} = add\n")).unwrap_err(),
            "line 1: snapshot cell count must be between 1 and 255"
        );
    }
    for count in ["0", "256", "-1", "null"] {
        assert!(serde_json::from_str::<NodeKind>(&format!(
            r#"{{"type":"snapshot","cells":{count}}}"#
        ))
        .is_err());
    }
    for port in ["c0", "c3", "unknown"] {
        assert!(dsl::parse(&format!(
            "cell input = 1\nsnapshot s cells 2 = add\ninput -> s.{port}\n"
        ))
        .is_err());
    }
    let doc = dsl::parse("stream input events 1:1\ncell state = 2\nsnapshot s cells 2 = add\ninput -> s.event\nstate -> s.c1\n").unwrap().doc;
    assert!(!sim::validate_document(&doc).errors.is_empty());
    assert!(
        dsl::parse("stream input events 1:1\nsnapshot s cells 2 = add\ninput -> s.c2\n").is_err()
    );
}

#[test]
fn snapshot_diagnostics_follow_existing_parser_and_validation_wording() {
    for (source, expected) in [
        ("snapshot s\n", "line 1: expected 'snapshot <name> = <op>'"),
        (
            "snapshot s cells 2\n",
            "line 1: expected 'snapshot <name> cells <count> = <op>'",
        ),
        (
            "snapshot s cells 2 = display \"処理\" extra\n",
            "line 1: unexpected text after display label",
        ),
        (
            "cell input = 1\nsnapshot s = add\ninput -> s.unknown\n",
            "line 3: unknown snapshot port 'unknown'",
        ),
        (
            "cell input = 1\nsnapshot s cells 2 = add\ninput -> s.c3\n",
            "line 3: node 's' has no port 3",
        ),
    ] {
        assert_eq!(dsl::parse(source).unwrap_err(), expected, "{source}");
    }
    let doc = dsl::parse(
        "stream input\ncell state = 1\nsnapshot s cells 2 = add\ninput -> s.event\nstate -> s.c1\n",
    )
    .unwrap()
    .doc;
    assert_eq!(
        sim::validate_document(&doc).errors,
        ["'s' has no cell input connected (port 3)"]
    );
    let mut doc = dsl::parse("stream input\nsnapshot s = add\ninput -> s.event\n")
        .unwrap()
        .doc;
    let node = doc.nodes.iter_mut().find(|node| node.name == "s").unwrap();
    if let NodeKind::Snapshot { cells, .. } = &mut node.kind {
        *cells = 0;
    }
    assert_eq!(
        sim::validate_document(&doc).errors,
        ["Snapshot 's' requires at least one cell input"]
    );
    assert!(
        serde_json::from_str::<NodeKind>(r#"{"type":"snapshot","cells":0}"#)
            .unwrap_err()
            .to_string()
            .starts_with("snapshot cell count must be between 1 and 255")
    );
}

#[test]
fn snapshot_folds_existing_operations_and_does_not_fire_on_cell_updates() {
    for (op, expected) in [
        ("add", "15"),
        ("mul", "60"),
        ("keep_left", "10"),
        ("keep_right", "2"),
        ("pair", "((10, 3), 2)"),
    ] {
        let doc = dsl::parse(&format!("stream input events 1:10\ncell a = 3 changes 2:4\ncell b = 2\nsnapshot s cells 2 = {op}\ninput -> s.event\na -> s.cell\nb -> s.c2\n")).unwrap().doc;
        let id = doc.nodes.iter().find(|node| node.name == "s").unwrap().id;
        let simulation = sim::simulate(&doc);
        assert_eq!(simulation.stream(id).len(), 1);
        assert_eq!(simulation.stream(id)[0].value.to_string(), expected);
    }
}

#[test]
fn snapshot_display_functions_are_structural_only() {
    let source = "stream input events 1:1\ncell a = 2\ncell b = 3\nsnapshot s cells 2 = display \"地図点を生成\"\ninput -> s.event\na -> s.c1\nb -> s.c2\n";
    let executable = dsl::parse(source).unwrap().doc;
    assert!(!sim::validate_document(&executable).errors.is_empty());
    let structural = dsl::parse(&format!("mode structural\n{source}"))
        .unwrap()
        .doc;
    assert!(sim::validate_document(&structural).errors.is_empty());
    assert!(render_svgs(&structural).network.contains("地図点を生成"));
}

#[test]
fn snapshot_example_passes_strict_cli_validation() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_frp-visualizer"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args(["check", "examples/snapshot-multi.frp", "--strict"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
