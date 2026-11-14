use frp_visualizer::{
    dsl, execution,
    model::{Document, Value},
    sim,
};

#[test]
fn combined_primitives_use_one_consistent_transaction_snapshot() {
    let doc = dsl::parse(include_str!("../examples/frp-primitives.frp"))
        .unwrap()
        .doc;
    let doc: Document = serde_json::from_str(&serde_json::to_string(&doc).unwrap()).unwrap();
    let id = |name| doc.nodes.iter().find(|node| node.name == name).unwrap().id;
    let report = sim::validate_document(&doc);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    let model = execution::build_execution_model(&doc);
    let out = model.lanes.iter().find(|lane| lane.label == "out").unwrap();
    let values: Vec<_> = model
        .stream(out)
        .iter()
        .map(|event| (event.time, event.value.to_string()))
        .collect();
    assert_eq!(values, [(1.0, "19".into()), (4.0, "53".into())]);
    let first = &model.sim.traces[0];
    assert_eq!(first.cell_before[&id("enabled")], Value::Bool(true));
    assert_eq!(first.cell_after[&id("enabled")], Value::Bool(false));
    assert_eq!(first.cell_before[&id("doubled")], Value::Number(4.0));
    assert_eq!(first.cell_after[&id("doubled")], Value::Number(6.0));
    assert_eq!(first.events[&id("total")], Value::Number(19.0));
}

#[test]
fn structural_map_on_cell_conditions_do_not_claim_reliable_gate_timing() {
    let doc = dsl::parse("mode structural\ncell source = true\nmap condition = display \"外部条件\"\nstream input events 1:42\ngate g\nsource -> condition\ncondition -> g.gate\ninput -> g.event\n").unwrap().doc;
    let report = sim::validate_document(&doc);
    assert!(report.errors.is_empty());
    assert!(!report.timing_reliable);
}

#[test]
fn combined_example_passes_strict_cli_validation() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_frp-visualizer"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args(["check", "examples/frp-primitives.frp", "--strict"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
