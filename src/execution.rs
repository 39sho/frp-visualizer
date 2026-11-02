use crate::model::{Document, NodeId, NodeKind, PortType};
use crate::sim::{simulate_with_mapping, FlattenedDocument, SimEvent, SimStep, Simulation};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ScopeId {
    Outer,
    Component(NodeId),
}

impl ScopeId {
    pub fn label(&self, doc: &Document) -> String {
        match self {
            Self::Outer => "outer".into(),
            Self::Component(id) => doc
                .node(*id)
                .map(|node| node.name.clone())
                .unwrap_or_else(|| format!("component-{id}")),
        }
    }
}

#[derive(Clone, Debug)]
pub struct LaneSpec {
    pub label: String,
    pub kind: PortType,
    pub source: NodeId,
    pub scope: ScopeId,
    pub position: [f32; 2],
}

#[derive(Clone, Debug)]
pub struct ExecutionModel {
    pub sim: Simulation,
    pub flat: FlattenedDocument,
    pub lanes: Vec<LaneSpec>,
    pub component_scopes: Vec<(NodeId, String)>,
}

impl ExecutionModel {
    pub fn lanes_for_scope(&self, scope: &ScopeId) -> Vec<LaneSpec> {
        let mut lanes: Vec<LaneSpec> = self
            .lanes
            .iter()
            .filter(|lane| &lane.scope == scope)
            .cloned()
            .collect();
        lanes.sort_by(|a, b| {
            a.position[0]
                .partial_cmp(&b.position[0])
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| {
                    a.position[1]
                        .partial_cmp(&b.position[1])
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .then_with(|| a.label.cmp(&b.label))
        });
        lanes
    }

    pub fn stream(&self, lane: &LaneSpec) -> &[SimEvent] {
        self.sim.stream(lane.source)
    }

    pub fn cell(&self, lane: &LaneSpec) -> &[SimStep] {
        self.sim.cell(lane.source)
    }
}

pub fn build_execution_model(doc: &Document) -> ExecutionModel {
    let (sim, flat) = simulate_with_mapping(doc);
    let mut lanes = Vec::new();
    let mut component_scopes = Vec::new();

    for node in &doc.nodes {
        if node.kind.is_event() || node.kind.is_cell() {
            let kind = if node.kind.is_cell() {
                PortType::Cell
            } else {
                PortType::Stream
            };
            lanes.push(LaneSpec {
                label: node.name.clone(),
                kind,
                source: node.id,
                scope: ScopeId::Outer,
                position: node.pos,
            });
        }
    }

    for node in &doc.nodes {
        if node.kind.is_sink() {
            if let Some(&(source, port)) = flat.sinks.get(&node.id) {
                if let Some(kind) = output_kind(&flat.doc, source, port) {
                    lanes.push(LaneSpec {
                        label: node.name.clone(),
                        kind,
                        source,
                        scope: ScopeId::Outer,
                        position: node.pos,
                    });
                }
            }
        }
    }

    for node in &doc.nodes {
        let NodeKind::Component { outputs, .. } = &node.kind else {
            continue;
        };
        for (index, port) in outputs.iter().enumerate() {
            if let Some(&(source, _)) = flat.component_outputs.get(&(node.id, index)) {
                if has_sink_downstream(doc, node.id, index) {
                    continue;
                }
                lanes.push(LaneSpec {
                    label: format!("{}.{}", node.name, port.name),
                    kind: port.ty,
                    source,
                    scope: ScopeId::Outer,
                    position: [
                        node.pos[0] + node.size()[0] + 20.0,
                        node.pos[1] + index as f32 * 28.0,
                    ],
                });
            }
        }
    }

    for node in &doc.nodes {
        let NodeKind::Component {
            inputs, outputs, ..
        } = &node.kind
        else {
            continue;
        };
        component_scopes.push((node.id, node.name.clone()));
        let scope = ScopeId::Component(node.id);
        for (index, port) in inputs.iter().enumerate() {
            if let Some(&(source, _)) = flat.component_inputs.get(&(node.id, index)) {
                lanes.push(LaneSpec {
                    label: format!("in {}", port.name),
                    kind: port.ty,
                    source,
                    scope: scope.clone(),
                    position: [-1_000_000.0 + index as f32, node.pos[1]],
                });
            }
        }
        if let Some(internal_ids) = flat.scopes.get(&node.id) {
            for id in internal_ids {
                let Some(internal) = flat.doc.node(*id) else {
                    continue;
                };
                if !internal.kind.is_event() && !internal.kind.is_cell() {
                    continue;
                }
                let kind = if internal.kind.is_cell() {
                    PortType::Cell
                } else {
                    PortType::Stream
                };
                lanes.push(LaneSpec {
                    label: short_label(&internal.name),
                    kind,
                    source: *id,
                    scope: scope.clone(),
                    position: internal.pos,
                });
            }
        }
        for (index, port) in outputs.iter().enumerate() {
            if let Some(&(source, _)) = flat.component_outputs.get(&(node.id, index)) {
                lanes.push(LaneSpec {
                    label: format!("out {}", port.name),
                    kind: port.ty,
                    source,
                    scope: scope.clone(),
                    position: [1_000_000.0 + index as f32, node.pos[1]],
                });
            }
        }
    }

    ExecutionModel {
        sim,
        flat,
        lanes,
        component_scopes,
    }
}

fn has_sink_downstream(doc: &Document, component: NodeId, port: usize) -> bool {
    let mut stack: Vec<NodeId> = doc
        .edges
        .iter()
        .filter(|edge| edge.from == component && edge.from_port == port)
        .map(|edge| edge.to)
        .collect();
    let mut seen = std::collections::HashSet::new();
    while let Some(node) = stack.pop() {
        if !seen.insert(node) {
            continue;
        }
        if doc
            .node(node)
            .map(|target| target.kind.is_sink())
            .unwrap_or(false)
        {
            return true;
        }
        for edge in doc.edges.iter().filter(|edge| edge.from == node) {
            stack.push(edge.to);
        }
    }
    false
}

fn output_kind(doc: &Document, node: NodeId, port: usize) -> Option<PortType> {
    doc.node(node)
        .and_then(|node| node.kind.output_types().get(port).copied())
}

fn short_label(name: &str) -> String {
    name.rsplit_once('/')
        .map(|(_, label)| label.to_string())
        .unwrap_or_else(|| name.to_string())
}

pub fn trace_json(doc: &Document, model: &ExecutionModel, timing_reliable: bool) -> String {
    let flat = &model.flat.doc;
    let name = |id: NodeId| {
        flat.node(id)
            .map(|node| node.name.clone())
            .unwrap_or_else(|| format!("#{id}"))
    };

    let transactions: Vec<serde_json::Value> = model
        .sim
        .traces
        .iter()
        .map(|trace| {
            let events: serde_json::Map<String, serde_json::Value> = trace
                .events
                .iter()
                .map(|(id, value)| (name(*id), serde_json::json!(value.to_string())))
                .collect();
            let updates: serde_json::Map<String, serde_json::Value> = trace
                .updates
                .iter()
                .map(|(id, value)| (name(*id), serde_json::json!(value.to_string())))
                .collect();
            let cell_before: serde_json::Map<String, serde_json::Value> = trace
                .cell_before
                .iter()
                .map(|(id, value)| (name(*id), serde_json::json!(value.to_string())))
                .collect();
            let cell_after: serde_json::Map<String, serde_json::Value> = trace
                .cell_after
                .iter()
                .map(|(id, value)| (name(*id), serde_json::json!(value.to_string())))
                .collect();
            serde_json::json!({
                "index": trace.index,
                "time": trace.time,
                "events": events,
                "updates": updates,
                "cell_before": cell_before,
                "cell_after": cell_after,
            })
        })
        .collect();

    let lanes: Vec<serde_json::Value> = model
        .lanes
        .iter()
        .map(|lane| {
            let kind = match lane.kind {
                PortType::Cell => "cell",
                PortType::Stream => "stream",
                PortType::Any => "any",
            };
            let mut value = serde_json::json!({
                "scope": lane.scope.label(doc),
                "label": lane.label,
                "kind": kind,
                "source": name(lane.source),
            });
            if lane.kind == PortType::Cell {
                let steps: Vec<serde_json::Value> = model
                    .sim
                    .cell(lane.source)
                    .iter()
                    .map(|step| {
                        serde_json::json!({
                            "time": step.time,
                            "value": step.value.to_string(),
                        })
                    })
                    .collect();
                value["steps"] = serde_json::Value::Array(steps);
            } else {
                let events: Vec<serde_json::Value> = model
                    .sim
                    .stream(lane.source)
                    .iter()
                    .map(|event| {
                        serde_json::json!({
                            "time": event.time,
                            "value": event.value.to_string(),
                        })
                    })
                    .collect();
                value["events"] = serde_json::Value::Array(events);
            }
            value
        })
        .collect();

    serde_json::to_string_pretty(&serde_json::json!({
        "title": doc.title,
        "timing_reliable": timing_reliable,
        "values_reliable": doc.mode == crate::model::DocumentMode::Executable,
        "transactions": transactions,
        "lanes": lanes,
    }))
    .unwrap_or_else(|_| "{}".to_string())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::render_timing::TimingOptions;
    use crate::RenderOptions;

    #[test]
    fn outer_scope_includes_sinks_and_skips_duplicate_component_outputs() {
        let doc = crate::examples::load_example("component").unwrap();
        let model = build_execution_model(&doc);
        let lanes = model.lanes_for_scope(&ScopeId::Outer);
        let labels: Vec<&str> = lanes.iter().map(|lane| lane.label.as_str()).collect();
        assert!(labels.contains(&"sDelta"));
        assert!(labels.contains(&"sClear"));
        assert!(labels.contains(&"observed"));
        assert!(labels.contains(&"total"));
        assert!(labels.contains(&"state"));
        assert!(!labels.iter().any(|label| label.starts_with("accumulate.")));
        for lane in &lanes {
            assert!(model.flat.doc.node(lane.source).is_some());
        }
    }

    #[test]
    fn component_scope_has_inputs_internal_signals_and_outputs() {
        let doc = crate::examples::load_example("component").unwrap();
        let component = doc
            .nodes
            .iter()
            .find(|node| node.kind.is_component())
            .unwrap()
            .id;
        let model = build_execution_model(&doc);
        let lanes = model.lanes_for_scope(&ScopeId::Component(component));
        let labels: Vec<&str> = lanes.iter().map(|lane| lane.label.as_str()).collect();
        for expected in ["in delta", "in clear", "out total", "out state"] {
            assert!(labels.contains(&expected), "missing {expected}: {labels:?}");
        }
        assert!(labels.contains(&"snapshot"));
        assert!(labels.contains(&"merge"));
        for lane in &lanes {
            assert!(model.flat.doc.node(lane.source).is_some());
        }
    }

    #[test]
    fn trace_json_contains_transactions_and_lanes() {
        let doc = crate::examples::load_example("component").unwrap();
        let model = build_execution_model(&doc);
        let trace = trace_json(&doc, &model, true);
        let value: serde_json::Value = serde_json::from_str(&trace).unwrap();
        assert_eq!(value["timing_reliable"], true);
        assert_eq!(value["values_reliable"], true);
        assert!(value["transactions"].as_array().unwrap().len() >= 3);
        assert!(!value["lanes"].as_array().unwrap().is_empty());
    }

    #[test]
    fn value_less_timing_omits_event_values() {
        let doc = crate::examples::load_example("component").unwrap();
        let model = build_execution_model(&doc);
        let options = RenderOptions {
            timing: TimingOptions {
                show_values: false,
                ..Default::default()
            },
            ..Default::default()
        };
        let (svg, _, _) =
            crate::render_timing_scope_svg(&doc, &model, &ScopeId::Outer, &options).unwrap();
        assert!(svg.contains(">T1</text>"));
        assert!(!svg.contains(">15</text>"));
    }
}
