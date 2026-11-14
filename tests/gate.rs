use frp_visualizer::{dsl, execution, layout, model::Document, render_svgs, sim};

#[test]
fn gate_uses_the_condition_before_the_transaction_and_preserves_values() {
    let doc = dsl::parse(include_str!("../examples/gate.frp"))
        .unwrap()
        .doc;
    let gate = doc
        .nodes
        .iter()
        .find(|node| node.name == "allowed")
        .unwrap();
    let simulation = sim::simulate(&doc);
    let values: Vec<_> = simulation
        .stream(gate.id)
        .iter()
        .map(|event| (event.time, event.value.to_string()))
        .collect();
    assert_eq!(
        values,
        vec![(2.0, "20".into()), (3.0, "30".into()), (5.0, "50".into())]
    );
    assert!(sim::validate_document(&doc).errors.is_empty());

    let json = serde_json::to_string(&doc).unwrap();
    let restored: Document = serde_json::from_str(&json).unwrap();
    assert_eq!(sim::simulate(&restored).stream(gate.id).len(), 3);
    let model = execution::build_execution_model(&restored);
    assert!(model.lanes.iter().any(|lane| lane.source == gate.id));
    let svg = render_svgs(&restored);
    assert!(svg.network.contains(">gate</text>"));
    assert!(svg.timing.contains("allowed"));

    let rect = gate.rect();
    assert_eq!(
        layout::input_port_pos(gate, rect, 0),
        [rect.x, rect.center()[1]]
    );
    assert_eq!(
        layout::input_port_pos(gate, rect, 1),
        [rect.center()[0], rect.bottom()]
    );
}

#[test]
fn condition_updates_do_not_create_events() {
    let doc = dsl::parse("stream input\ncell enabled = false changes 1:true, 2:false\ngate g\ninput -> g.event\nenabled -> g.cell\n").unwrap().doc;
    let gate = doc.nodes.iter().find(|node| node.name == "g").unwrap();
    assert!(sim::simulate(&doc).stream(gate.id).is_empty());
}

#[test]
fn gate_works_inside_an_expanded_component() {
    let doc = dsl::parse("component controlled {\n in input: stream\n in enabled: cell\n out result: stream\n gate g\n input -> g.event\n enabled -> g.gate\n g -> result\n}\nstream input events 1:42\ncell enabled = true\ncomponent c = controlled\ninput -> c.input\nenabled -> c.enabled\nsink out\nc.result -> out\n").unwrap().doc;
    let report = sim::validate_document(&doc);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let model = execution::build_execution_model(&doc);
    let output = model.lanes.iter().find(|lane| lane.label == "out").unwrap();
    assert_eq!(model.stream(output)[0].value.to_string(), "42");
}

#[test]
fn gate_rejects_non_boolean_conditions_and_missing_ports() {
    for condition in ["1", "true changes 1:2"] {
        let doc = dsl::parse(&format!("stream input\ncell enabled = {condition}\ngate g\ninput -> g.event\nenabled -> g.gate\n")).unwrap().doc;
        assert_eq!(
            sim::validate_document(&doc).errors,
            ["Gate 'g' requires a Bool condition cell"]
        );
    }
    let doc = dsl::parse("stream input\ngate g\ninput -> g.event\n")
        .unwrap()
        .doc;
    assert_eq!(
        sim::validate_document(&doc).errors,
        ["'g' has no cell input connected (port 2)"]
    );
    assert_eq!(
        dsl::parse("gate g = true\n").unwrap_err(),
        "line 1: gate does not take a value"
    );
    assert_eq!(
        dsl::parse("cell enabled = true\ngate g\nenabled -> g.unknown\n").unwrap_err(),
        "line 3: unknown gate port 'unknown'"
    );
    assert_eq!(
        dsl::parse("stream input events 1:true\ngate g\ninput -> g.gate\n").unwrap_err(),
        "line 3: stream output cannot connect to cell input"
    );
}

#[test]
fn display_only_conditions_make_gate_timing_unreliable() {
    let doc = dsl::parse("mode structural\nstream input events 1:1\nmap unknown = display \"条件判定\"\nhold enabled = false\ngate g\ninput -> unknown\nunknown -> enabled\ninput -> g.event\nenabled -> g.gate\n").unwrap().doc;
    let report = sim::validate_document(&doc);
    assert!(report.errors.is_empty());
    assert!(!report.timing_reliable);
    assert_eq!(
        report.warnings,
        ["Timing event flow may depend on display-only values through gate 'g'"]
    );
}

#[test]
fn gate_example_passes_strict_cli_validation() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_frp-visualizer"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args(["check", "examples/gate.frp", "--strict"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
