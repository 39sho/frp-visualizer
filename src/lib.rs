pub mod dsl;
pub mod examples;
pub mod execution;
pub mod layout;
pub mod model;
pub mod render_annotation;
pub mod render_network;
pub mod render_timing;
pub mod scene;
pub mod sim;

use layout::{layout_with, LayoutOptions};
use model::{ComponentDef, Document, DocumentMode};
use render_network::{draw_notation_key, render_network};
use render_timing::{render_timing, TimingOptions};
use scene::{Scene, TextAnchor, INK, MUTED};

pub struct SvgBundle {
    pub network: String,
    pub timing: String,
    pub combined: String,
}

#[derive(Clone, Debug, Default)]
pub struct RenderOptions {
    pub timing: TimingOptions,
    pub layout: LayoutOptions,
}

pub fn render_svgs(doc: &Document) -> SvgBundle {
    render_svgs_with_options(doc, &RenderOptions::default())
}

pub fn render_svgs_with(doc: &Document, timing_options: &TimingOptions) -> SvgBundle {
    render_svgs_with_options(
        doc,
        &RenderOptions {
            timing: timing_options.clone(),
            layout: LayoutOptions::default(),
        },
    )
}

pub fn render_svgs_with_options(doc: &Document, options: &RenderOptions) -> SvgBundle {
    let model = execution::build_execution_model(doc);
    let outer_lanes = model.lanes_for_scope(&execution::ScopeId::Outer);
    let timing_options = effective_timing_options(doc, &options.timing);
    let network_layout = layout_with(doc, &options.layout);
    let net_scene = render_network(doc, &network_layout);
    let net_bounds = net_scene
        .bounds()
        .unwrap_or(model::Rect::new(0.0, 0.0, 800.0, 400.0));
    let mut legend = Scene::new();
    let legend_w = draw_notation_key(&mut legend, 18.0, 18.0);
    let legend_h = 30.0;
    let mut net = net_scene.clone();
    net.translate(-net_bounds.x + 18.0, -net_bounds.y + 18.0 + legend_h);
    let net_w = (net_bounds.w + 36.0).max(legend_w + 36.0);
    let net_h = net_bounds.h + 36.0 + legend_h;
    legend.append(&net, 0.0, 0.0);
    let network = legend.to_svg(net_w, net_h, 0.0);

    let timing_view = render_timing(
        doc,
        &model.sim,
        &outer_lanes,
        &timing_options,
        true,
        &model.flat.doc,
    );
    let timing = timing_view
        .scene
        .to_svg(timing_view.width, timing_view.height, 0.0);

    let combined = render_combined(doc, &legend, net_w, net_h, &timing_view);

    SvgBundle {
        network,
        timing,
        combined,
    }
}

fn effective_timing_options(doc: &Document, options: &TimingOptions) -> TimingOptions {
    let mut timing = options.clone();
    if doc.mode == DocumentMode::Structural {
        timing.show_values = false;
        timing.focus = None;
    }
    timing
}

fn render_combined(
    doc: &Document,
    net: &Scene,
    net_w: f32,
    net_h: f32,
    timing: &render_timing::TimingView,
) -> String {
    let margin = 24.0;
    let header_h = 58.0;
    let section_gap = 26.0;
    let width = net_w.max(timing.width) + margin * 2.0;
    let height = header_h + net_h + section_gap + timing.height + margin;
    let mut scene = Scene::new();

    scene.bold_text([margin, 28.0], &doc.title, 19.0, INK, TextAnchor::Start);
    scene.text(
        [margin, 48.0],
        "FRP network and timing diagram",
        10.5,
        MUTED,
        TextAnchor::Start,
    );

    scene.text(
        [margin, header_h + 2.0],
        "NETWORK",
        10.5,
        MUTED,
        TextAnchor::Start,
    );
    scene.append(net, margin, header_h + 12.0);

    let timing_y = header_h + net_h + section_gap + 12.0;
    scene.text(
        [margin, timing_y - 14.0],
        "TIMING",
        10.5,
        MUTED,
        TextAnchor::Start,
    );
    scene.append(&timing.scene, margin, timing_y);

    scene.to_svg(width, height, 0.0)
}

pub fn find_component<'a>(doc: &'a Document, name: &str) -> Option<&'a ComponentDef> {
    for definition in &doc.components {
        if definition.name == name {
            return Some(definition);
        }
        if let Some(found) = find_component(&definition.doc, name) {
            return Some(found);
        }
    }
    None
}

pub fn render_timing_scope_svg(
    doc: &Document,
    model: &execution::ExecutionModel,
    scope: &execution::ScopeId,
    options: &RenderOptions,
) -> Option<(String, f32, f32)> {
    let lanes = model.lanes_for_scope(scope);
    if lanes.is_empty() {
        return None;
    }
    let timing_options = effective_timing_options(doc, &options.timing);
    let view = render_timing(
        doc,
        &model.sim,
        &lanes,
        &timing_options,
        false,
        &model.flat.doc,
    );
    Some((
        view.scene.to_svg(view.width, view.height, 0.0),
        view.width,
        view.height,
    ))
}

pub fn render_component_network(
    doc: &Document,
    name: &str,
    options: &RenderOptions,
) -> Option<String> {
    let definition = find_component(doc, name)?;
    Some(render_svgs_with_options(&definition.doc, options).network)
}

pub fn export_svgs_to_dir(
    doc: &Document,
    dir: &std::path::Path,
) -> std::io::Result<Vec<std::path::PathBuf>> {
    export_svgs_to_dir_with(doc, dir, &RenderOptions::default())
}

pub fn export_svgs_to_dir_with(
    doc: &Document,
    dir: &std::path::Path,
    options: &RenderOptions,
) -> std::io::Result<Vec<std::path::PathBuf>> {
    std::fs::create_dir_all(dir)?;
    let bundle = render_svgs_with_options(doc, options);
    let mut paths = Vec::new();
    for (name, contents) in [
        ("network.svg", bundle.network),
        ("timing.svg", bundle.timing),
        ("diagram.svg", bundle.combined),
    ] {
        let path = dir.join(name);
        std::fs::write(&path, contents)?;
        paths.push(path);
    }
    Ok(paths)
}

pub fn save_document(doc: &Document, path: &std::path::Path) -> Result<(), String> {
    let json = serde_json::to_string_pretty(doc).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())
}

pub fn load_document(path: &std::path::Path) -> Result<Document, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CombineOp, NodeKind, PortSpec, PortType, TimedValue, UnaryOp, Value};

    #[test]
    fn every_example_renders_all_svgs() {
        for (label, key) in examples::EXAMPLES {
            let doc = examples::load_example(key).unwrap();
            let bundle = render_svgs(&doc);
            for svg in [&bundle.network, &bundle.timing, &bundle.combined] {
                assert!(svg.starts_with("<svg"), "{label}: missing svg");
                assert!(svg.ends_with("</svg>\n"), "{label}: truncated svg");
                assert!(svg.len() > 1500, "{label}: suspiciously small svg");
            }
            let escaped = doc
                .title
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;");
            assert!(bundle.combined.contains(&escaped), "{label}: title missing");
        }
    }

    #[test]
    fn diagrams_use_the_compact_notation() {
        let doc = examples::load_example("accumulator").unwrap();
        let bundle = render_svgs(&doc);
        for needle in [
            ">stream</text>",
            ">cell</text>",
            ">snapshot</text>",
            ">hold</text>",
            ">+</text>",
            ">sDelta</text>",
            ">total</text>",
            "stream wire",
            "cell wire",
            "#181818",
            "#4a799e",
            "stroke-dasharray",
        ] {
            assert!(bundle.network.contains(needle), "network missing {needle}");
        }
        assert!(bundle.timing.contains("Tn start"));
        assert!(bundle.timing.contains("events in order"));
        assert!(bundle.timing.contains("solid: event flow"));
        assert!(bundle.timing.contains("dashed: cell dependency"));
        assert!(bundle.timing.contains(">after</text>"));
        assert!(bundle.timing.contains(">#1</text>"));
        assert!(bundle.timing.contains(">T1</text>"));

        let filter_doc = examples::load_example("map_filter").unwrap();
        let filter_bundle = render_svgs(&filter_doc);
        assert!(filter_bundle.network.contains(">filter</text>"));
    }

    #[test]
    fn component_size_reserves_space_for_ports() {
        use crate::model::{Node, PortSpec, PortType};
        let inputs = vec![
            PortSpec {
                name: "sFrame".into(),
                ty: PortType::Stream,
            },
            PortSpec {
                name: "cLocalKFsBuffer".into(),
                ty: PortType::Cell,
            },
            PortSpec {
                name: "cReferenceKF".into(),
                ty: PortType::Cell,
            },
        ];
        let outputs = vec![
            PortSpec {
                name: "frameUpdated".into(),
                ty: PortType::Stream,
            },
            PortSpec {
                name: "localKFsUpdated".into(),
                ty: PortType::Stream,
            },
            PortSpec {
                name: "referenceKFUpdated".into(),
                ty: PortType::Stream,
            },
            PortSpec {
                name: "forLocalPoints".into(),
                ty: PortType::Stream,
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
        let [width, height] = node.size();
        assert!(width > 300.0, "width was {width}");
        assert!(height > 108.0, "height was {height}");
        let _ = node.name;
    }

    #[test]
    fn annotation_callout_renders() {
        let doc = examples::load_example("annotation").unwrap();
        let bundle = render_svgs(&doc);
        assert!(bundle.network.contains("each event is incremented by one"));
        assert!(bundle.network.contains("#9ac658"));
    }

    #[test]
    fn transaction_annotation_renders_in_timing() {
        let mut doc = Document::blank();
        let source = doc.add_node(
            NodeKind::InputEvent {
                events: vec![TimedValue::new(1.0, Value::Number(1.0))],
            },
            [0.0, 0.0],
        );
        let map = doc.add_node(
            NodeKind::Map {
                op: UnaryOp::Identity,
            },
            [200.0, 0.0],
        );
        let sink = doc.add_node(NodeKind::Sink, [400.0, 0.0]);
        doc.add_edge(source, map, 0).unwrap();
        doc.add_edge(map, sink, 0).unwrap();
        doc.annotations.push(model::Annotation {
            text: "transaction T1".into(),
            target: Some(model::AnnotationTarget::Transaction { index: 0 }),
            position: Some([220.0, 70.0]),
        });
        let bundle = render_svgs(&doc);
        assert!(bundle.timing.contains("transaction T1"));
        assert!(bundle.timing.contains("#9ac658"));
    }

    #[test]
    fn component_network_can_be_rendered_expanded() {
        let doc = examples::load_example("component").unwrap();
        let svg = render_component_network(&doc, "accumulate", &RenderOptions::default())
            .expect("expanded network");
        assert!(svg.starts_with("<svg"));
        assert!(svg.contains(">snapshot</text>"));
        assert!(svg.contains(">merge</text>"));
        assert!(svg.contains(">hold</text>"));
        assert!(render_component_network(&doc, "missing", &RenderOptions::default()).is_none());
    }

    #[test]
    fn focused_timing_embeds_the_execution_panel() {
        let doc = examples::load_example("accumulator").unwrap();
        let options = render_timing::TimingOptions {
            focus: Some(0),
            ..Default::default()
        };
        let bundle = render_svgs_with(&doc, &options);
        assert!(bundle.timing.contains("Execution in time"));
        assert!(bundle.timing.contains("snap!"));
        assert!(bundle.timing.contains("update"));
    }

    #[test]
    fn auto_layout_keeps_feedback_after_the_accumulator() {
        let mut doc = examples::load_example("calibration").unwrap();
        layout::auto_layout(&mut doc);
        let x = |name: &str| doc.nodes.iter().find(|node| node.name == name).unwrap().pos[0];
        assert!(x("snapshot") < x("merge"));
        assert!(x("merge") < x("hold"));
        assert!(x("hold") < x("lift"));
    }

    #[test]
    fn simulation_records_transaction_traces() {
        let doc = examples::load_example("accumulator").unwrap();
        let sim = sim::simulate(&doc);
        assert_eq!(sim.traces.len(), 4);
        let snapshot = doc.nodes.iter().find(|n| n.name == "snapshot").unwrap().id;
        let hold = doc.nodes.iter().find(|n| n.name == "accum").unwrap().id;
        let first = &sim.traces[0];
        assert_eq!(first.time, 1.0);
        assert_eq!(
            first.events.get(&snapshot).map(ToString::to_string),
            Some("5".to_string())
        );
        assert_eq!(
            first.cell_before.get(&hold).map(ToString::to_string),
            Some("0".to_string())
        );
        assert_eq!(
            first.cell_after.get(&hold).map(ToString::to_string),
            Some("5".to_string())
        );
    }

    #[test]
    fn documents_round_trip_through_json() {
        for (_, key) in examples::EXAMPLES {
            let doc = examples::load_example(key).unwrap();
            let json = serde_json::to_string_pretty(&doc).unwrap();
            let back: Document = serde_json::from_str(&json).unwrap();
            let a = serde_json::to_value(&doc).unwrap();
            let b = serde_json::to_value(&back).unwrap();
            assert_eq!(a, b, "{key}: JSON round trip changed the document");
            let sim_a = sim::simulate(&doc);
            let sim_b = sim::simulate(&back);
            assert_eq!(sim_a.warnings, sim_b.warnings);
            for node in &doc.nodes {
                let ea: Vec<String> = sim_a
                    .stream(node.id)
                    .iter()
                    .map(|e| e.value.to_string())
                    .collect();
                let eb: Vec<String> = sim_b
                    .stream(node.id)
                    .iter()
                    .map(|e| e.value.to_string())
                    .collect();
                assert_eq!(ea, eb, "{key}: stream '{}' changed", node.name);
                let ca: Vec<String> = sim_a
                    .cell(node.id)
                    .iter()
                    .map(|e| e.value.to_string())
                    .collect();
                let cb: Vec<String> = sim_b
                    .cell(node.id)
                    .iter()
                    .map(|e| e.value.to_string())
                    .collect();
                assert_eq!(ca, cb, "{key}: cell '{}' changed", node.name);
            }
        }
    }

    #[test]
    fn svg_export_writes_three_files() {
        let doc = examples::load_example("accumulator").unwrap();
        let dir = std::env::temp_dir().join(format!("frp_visualizer_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let paths = export_svgs_to_dir(&doc, &dir).unwrap();
        assert_eq!(paths.len(), 3);
        for path in &paths {
            let text = std::fs::read_to_string(path).unwrap();
            assert!(text.starts_with("<svg"));
        }
        assert!(dir.join("diagram.svg").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn json_escaping_is_safe_in_svg() {
        let mut doc = Document::blank();
        doc.title = "<unsafe & title>".into();
        let id = doc.add_node(
            NodeKind::Map {
                op: UnaryOp::Add(1.0),
            },
            [0.0, 0.0],
        );
        if let Some(node) = doc.node_mut(id) {
            node.name = "<map>".into();
        }
        let bundle = render_svgs(&doc);
        assert!(bundle.combined.contains("&lt;unsafe &amp; title&gt;"));
        assert!(!bundle.combined.contains("<unsafe"));
    }

    #[test]
    fn hold_and_snapshot_use_special_ports() {
        let mut doc = Document::blank();
        let snap = doc.add_node(NodeKind::Snapshot { op: CombineOp::Add }, [100.0, 100.0]);
        let hold = doc.add_node(
            NodeKind::Hold {
                initial: Value::Number(0.0),
            },
            [100.0, 300.0],
        );
        let lay = crate::layout::layout(&doc);
        let snap_rect = lay.node_rects[&snap];
        let hold_rect = lay.node_rects[&hold];
        let p = lay.input_port(&doc, snap, 1).unwrap();
        assert!((p[0] - snap_rect.center()[0]).abs() < 0.1);
        assert!((p[1] - snap_rect.bottom()).abs() < 0.1);
        let p = lay.input_port(&doc, hold, 0).unwrap();
        assert!((p[0] - hold_rect.center()[0]).abs() < 0.1);
        assert!((p[1] - hold_rect.y).abs() < 0.1);
    }

    #[test]
    fn edge_validation_rejects_bad_connections() {
        let mut doc = Document::blank();
        let event = doc.add_node(
            NodeKind::InputEvent {
                events: vec![TimedValue::new(1.0, Value::Number(1.0))],
            },
            [0.0, 0.0],
        );
        let cell = doc.add_node(
            NodeKind::InputCell {
                initial: Value::Number(0.0),
                changes: vec![],
            },
            [0.0, 100.0],
        );
        let map = doc.add_node(
            NodeKind::Map {
                op: UnaryOp::Identity,
            },
            [200.0, 0.0],
        );
        let lift = doc.add_node(NodeKind::Lift { op: CombineOp::Add }, [400.0, 0.0]);
        assert!(doc.validate_edge(event, map, 0).is_ok());
        assert!(doc.validate_edge(cell, map, 0).is_err());
        assert!(doc.validate_edge(event, event, 0).is_err());
        assert!(doc.validate_edge(cell, lift, 0).is_ok());
        doc.add_edge(cell, lift, 0).unwrap();
        assert!(doc.validate_edge(cell, lift, 0).is_err());
        assert!(doc.validate_edge(event, lift, 0).is_err());

        let component = doc.add_node(
            NodeKind::Component {
                definition: None,
                inputs: vec![PortSpec {
                    name: "in".into(),
                    ty: PortType::Stream,
                }],
                outputs: vec![
                    PortSpec {
                        name: "a".into(),
                        ty: PortType::Stream,
                    },
                    PortSpec {
                        name: "b".into(),
                        ty: PortType::Stream,
                    },
                ],
                body: Vec::new(),
            },
            [600.0, 0.0],
        );
        assert!(doc.validate_edge_from(component, 1, map, 0).is_ok());
        assert!(doc.validate_edge_from(component, 2, map, 0).is_err());
    }
}
