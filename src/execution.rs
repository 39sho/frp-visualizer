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
pub enum SignalId {
    Node(NodeId),
    ComponentInput { component: NodeId, port: usize },
    ComponentOutput { component: NodeId, port: usize },
    Sink(NodeId),
    Internal(NodeId),
}

#[derive(Clone, Debug)]
pub struct LaneSpec {
    pub signal: SignalId,
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
                signal: SignalId::Node(node.id),
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
                        signal: SignalId::Sink(node.id),
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
                    signal: SignalId::ComponentOutput {
                        component: node.id,
                        port: index,
                    },
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
                    signal: SignalId::ComponentInput {
                        component: node.id,
                        port: index,
                    },
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
                    signal: SignalId::Internal(*id),
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
                    signal: SignalId::ComponentOutput {
                        component: node.id,
                        port: index,
                    },
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
