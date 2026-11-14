use crate::model::{
    ComponentDef, Document, DocumentMode, Edge, FilterOp, NodeId, NodeKind, PortSpec, PortType,
    Value,
};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug)]
pub struct SimEvent {
    pub time: f64,
    pub value: Value,
}

#[derive(Clone, Debug)]
pub struct SimStep {
    pub time: f64,
    pub value: Value,
}

#[derive(Clone, Debug)]
pub struct TransactionTrace {
    pub index: usize,
    pub time: f64,
    pub events: HashMap<NodeId, Value>,
    pub event_order: Vec<NodeId>,
    pub cell_before: HashMap<NodeId, Value>,
    pub cell_after: HashMap<NodeId, Value>,
    pub updates: HashMap<NodeId, Value>,
    pub update_order: Vec<NodeId>,
}

#[derive(Clone, Debug, Default)]
pub struct FlattenedDocument {
    pub doc: Document,
    pub warnings: Vec<String>,
    pub component_inputs: HashMap<(NodeId, usize), (NodeId, usize)>,
    pub component_outputs: HashMap<(NodeId, usize), (NodeId, usize)>,
    pub scopes: HashMap<NodeId, Vec<NodeId>>,
    pub scope_parents: HashMap<NodeId, Option<NodeId>>,
    pub scope_labels: HashMap<NodeId, String>,
    pub scope_ports: HashMap<NodeId, (Vec<PortSpec>, Vec<PortSpec>)>,
    pub sinks: HashMap<NodeId, (NodeId, usize)>,
}

#[derive(Clone, Debug, Default)]
pub struct Simulation {
    pub streams: HashMap<NodeId, Vec<SimEvent>>,
    pub cells: HashMap<NodeId, Vec<SimStep>>,
    pub transactions: Vec<f64>,
    pub traces: Vec<TransactionTrace>,
    pub warnings: Vec<String>,
    pub end_time: f64,
}

impl Simulation {
    pub fn stream(&self, node: NodeId) -> &[SimEvent] {
        self.streams.get(&node).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn cell(&self, node: NodeId) -> &[SimStep] {
        self.cells.get(&node).map(Vec::as_slice).unwrap_or(&[])
    }
}

const MAX_COMPONENT_EXPANSIONS: usize = 64;

pub fn flatten_components(doc: &Document) -> (Document, Vec<String>) {
    let flattened = flatten_document(doc);
    (flattened.doc, flattened.warnings)
}

pub fn flatten_document(doc: &Document) -> FlattenedDocument {
    let mut definitions = std::collections::HashMap::new();
    let mut warnings = Vec::new();
    collect_component_defs(doc, &mut definitions, &mut warnings);
    let mut flat = doc.clone();
    flat.components.clear();
    let original_nodes: HashSet<NodeId> = doc.nodes.iter().map(|node| node.id).collect();
    let mut context = FlattenContext {
        definitions,
        warnings,
        next_node_id: flat.nodes.iter().map(|node| node.id).max().unwrap_or(0) + 1,
        next_edge_id: flat.edges.iter().map(|edge| edge.id).max().unwrap_or(0) + 1,
        processed: HashSet::new(),
        expansions: 0,
        original_nodes,
        owner_of: HashMap::new(),
        component_inputs_pending: HashMap::new(),
        component_outputs: HashMap::new(),
        scopes: HashMap::new(),
        scope_parents: HashMap::new(),
        scope_labels: HashMap::new(),
        scope_ports: HashMap::new(),
        sinks: HashMap::new(),
    };
    context.expand_all(&mut flat);
    context.resolve_mappings(&flat, doc);
    FlattenedDocument {
        doc: flat,
        warnings: context.warnings,
        component_inputs: context.component_inputs_pending,
        component_outputs: context.component_outputs,
        scopes: context.scopes,
        scope_parents: context.scope_parents,
        scope_labels: context.scope_labels,
        scope_ports: context.scope_ports,
        sinks: context.sinks,
    }
}

fn collect_component_defs(
    doc: &Document,
    definitions: &mut std::collections::HashMap<String, ComponentDef>,
    warnings: &mut Vec<String>,
) {
    for definition in &doc.components {
        if definitions.contains_key(&definition.name) {
            warnings.push(format!(
                "Duplicate component definition '{}'; keeping the first one",
                definition.name
            ));
        } else {
            definitions.insert(definition.name.clone(), definition.clone());
        }
        collect_component_defs(&definition.doc, definitions, warnings);
    }
}

struct FlattenContext {
    definitions: std::collections::HashMap<String, ComponentDef>,
    warnings: Vec<String>,
    next_node_id: NodeId,
    next_edge_id: u64,
    processed: HashSet<NodeId>,
    expansions: usize,
    original_nodes: HashSet<NodeId>,
    owner_of: HashMap<NodeId, NodeId>,
    component_inputs_pending: HashMap<(NodeId, usize), (NodeId, usize)>,
    component_outputs: HashMap<(NodeId, usize), (NodeId, usize)>,
    scopes: HashMap<NodeId, Vec<NodeId>>,
    scope_parents: HashMap<NodeId, Option<NodeId>>,
    scope_labels: HashMap<NodeId, String>,
    scope_ports: HashMap<NodeId, (Vec<PortSpec>, Vec<PortSpec>)>,
    sinks: HashMap<NodeId, (NodeId, usize)>,
}

impl FlattenContext {
    fn expand_all(&mut self, doc: &mut Document) {
        loop {
            if self.expansions >= MAX_COMPONENT_EXPANSIONS {
                self.warnings.push(format!(
                    "Component expansion limit reached ({MAX_COMPONENT_EXPANSIONS}); remaining components are opaque"
                ));
                break;
            }
            let candidate = doc
                .nodes
                .iter()
                .find(|node| node.kind.is_component() && !self.processed.contains(&node.id))
                .cloned();
            let Some(component) = candidate else {
                break;
            };
            self.processed.insert(component.id);
            if self.expand_component(doc, &component) {
                self.expansions += 1;
            } else {
                self.warnings.push(format!(
                    "Component '{}' has no definition; leaving it opaque",
                    component.name
                ));
            }
        }
    }

    fn resolve_mappings(&mut self, flat: &Document, source: &Document) {
        let output_keys: Vec<(NodeId, usize)> = self.component_outputs.keys().copied().collect();
        for key in output_keys {
            let (mut node, mut port) = self.component_outputs[&key];
            for _ in 0..MAX_COMPONENT_EXPANSIONS {
                if let Some(&(next_node, next_port)) = self.component_outputs.get(&(node, port)) {
                    if next_node == node && next_port == port {
                        break;
                    }
                    node = next_node;
                    port = next_port;
                } else {
                    break;
                }
            }
            self.component_outputs.insert(key, (node, port));
        }

        let input_keys: Vec<(NodeId, usize)> =
            self.component_inputs_pending.keys().copied().collect();
        for key in input_keys {
            let (mut node, mut port) = self.component_inputs_pending[&key];
            for _ in 0..MAX_COMPONENT_EXPANSIONS {
                if let Some(&(next_node, next_port)) = self.component_outputs.get(&(node, port)) {
                    if next_node == node && next_port == port {
                        break;
                    }
                    node = next_node;
                    port = next_port;
                } else {
                    break;
                }
            }
            self.component_inputs_pending.insert(key, (node, port));
        }

        for node in &source.nodes {
            if node.kind.is_sink() {
                if let Some(edge) = flat.edges.iter().find(|edge| edge.to == node.id) {
                    self.sinks.insert(node.id, (edge.from, edge.from_port));
                }
            }
        }
    }

    fn expand_component(&mut self, doc: &mut Document, component: &crate::model::Node) -> bool {
        let NodeKind::Component {
            definition,
            inputs,
            outputs,
            ..
        } = &component.kind
        else {
            return false;
        };
        let Some(definition_name) = definition else {
            return false;
        };
        let Some(definition) = self.definitions.get(definition_name).cloned() else {
            return false;
        };
        let input_ports: &[PortSpec] = if inputs.is_empty() {
            &definition.inputs
        } else {
            inputs
        };
        let output_ports: &[PortSpec] = if outputs.is_empty() {
            &definition.outputs
        } else {
            outputs
        };
        let parent = if self.original_nodes.contains(&component.id) {
            None
        } else {
            self.owner_of.get(&component.id).copied()
        };
        self.scope_parents.insert(component.id, parent);
        let scope_label = component
            .name
            .rsplit_once('/')
            .map(|(_, label)| label.to_string())
            .unwrap_or_else(|| component.name.clone());
        self.scope_labels.insert(component.id, scope_label);
        self.scope_ports
            .insert(component.id, (input_ports.to_vec(), output_ports.to_vec()));

        let mut id_map: HashMap<NodeId, NodeId> = HashMap::new();
        for node in &definition.doc.nodes {
            id_map.insert(node.id, self.next_node_id);
            self.next_node_id += 1;
        }
        let prefix = if component.name.is_empty() {
            format!("#{}", component.id)
        } else {
            component.name.clone()
        };
        for node in &definition.doc.nodes {
            let mut cloned = node.clone();
            cloned.id = id_map[&node.id];
            cloned.name = format!("{prefix}/{}", cloned.name);
            self.owner_of.insert(cloned.id, component.id);
            self.scopes.entry(component.id).or_default().push(cloned.id);
            doc.nodes.push(cloned);
        }
        let mut internal_edges = Vec::new();
        for edge in &definition.doc.edges {
            let (Some(&from), Some(&to)) = (id_map.get(&edge.from), id_map.get(&edge.to)) else {
                continue;
            };
            let id = self.next_edge_id;
            self.next_edge_id += 1;
            internal_edges.push(Edge {
                id,
                from,
                from_port: edge.from_port,
                to,
                to_port: edge.to_port,
                label: edge.label.clone(),
                label_width: edge.label_width,
            });
        }
        doc.edges.extend(internal_edges);

        let mut input_nodes: HashMap<String, (NodeId, Option<PortType>)> = HashMap::new();
        let mut output_nodes: HashMap<String, NodeId> = HashMap::new();
        for node in &definition.doc.nodes {
            match &node.kind {
                NodeKind::InputEvent { .. } | NodeKind::InputCell { .. } => {
                    input_nodes.insert(
                        node.name.clone(),
                        (id_map[&node.id], node.kind.output_types().first().copied()),
                    );
                }
                NodeKind::Sink => {
                    output_nodes.insert(node.name.clone(), id_map[&node.id]);
                }
                _ => {}
            }
        }

        for (port_index, port) in input_ports.iter().enumerate() {
            let Some((internal_id, source_type)) = input_nodes.get(&port.name).copied() else {
                self.warnings.push(format!(
                    "Component '{}' input '{}' has no matching internal input node",
                    component.name, port.name
                ));
                continue;
            };
            if let Some(source_type) = source_type {
                if source_type != port.ty {
                    self.warnings.push(format!(
                        "Component '{}' input '{}' expects {} but the internal node is {}",
                        component.name,
                        port.name,
                        port.ty.name(),
                        source_type.name()
                    ));
                }
            }
            let outer_edge = doc
                .edges
                .iter()
                .find(|edge| edge.to == component.id && edge.to_port == port_index)
                .cloned();
            if let Some(outer_edge) = outer_edge {
                self.component_inputs_pending.insert(
                    (component.id, port_index),
                    (outer_edge.from, outer_edge.from_port),
                );
                for edge in doc.edges.iter_mut().filter(|edge| edge.from == internal_id) {
                    edge.from = outer_edge.from;
                    edge.from_port = outer_edge.from_port;
                }
                doc.nodes.retain(|node| node.id != internal_id);
            } else {
                self.warnings.push(format!(
                    "Component '{}' input '{}' is not connected",
                    component.name, port.name
                ));
                doc.nodes.retain(|node| node.id != internal_id);
                doc.edges
                    .retain(|edge| edge.from != internal_id && edge.to != internal_id);
            }
        }

        for (port_index, port) in output_ports.iter().enumerate() {
            let Some(&sink_id) = output_nodes.get(&port.name) else {
                self.warnings.push(format!(
                    "Component '{}' output '{}' has no matching internal sink",
                    component.name, port.name
                ));
                doc.edges
                    .retain(|edge| !(edge.from == component.id && edge.from_port == port_index));
                continue;
            };
            let internal_edge = doc.edges.iter().find(|edge| edge.to == sink_id).cloned();
            if let Some(internal_edge) = internal_edge {
                let producer = internal_edge.from;
                let producer_port = internal_edge.from_port;
                self.component_outputs
                    .insert((component.id, port_index), (producer, producer_port));
                if let Some(producer_type) = doc
                    .node(producer)
                    .and_then(|node| node.kind.output_types().get(producer_port).copied())
                {
                    if producer_type != port.ty {
                        self.warnings.push(format!(
                            "Component '{}' output '{}' expects {} but the internal producer is {}",
                            component.name,
                            port.name,
                            port.ty.name(),
                            producer_type.name()
                        ));
                    }
                }
                for edge in doc
                    .edges
                    .iter_mut()
                    .filter(|edge| edge.from == component.id && edge.from_port == port_index)
                {
                    edge.from = producer;
                    edge.from_port = producer_port;
                }
                doc.edges.retain(|edge| edge.id != internal_edge.id);
            } else {
                self.warnings.push(format!(
                    "Component '{}' output '{}' has no internal producer",
                    component.name, port.name
                ));
                doc.edges
                    .retain(|edge| !(edge.from == component.id && edge.from_port == port_index));
            }
            doc.nodes.retain(|node| node.id != sink_id);
            doc.edges
                .retain(|edge| edge.from != sink_id && edge.to != sink_id);
        }

        doc.nodes.retain(|node| node.id != component.id);
        doc.edges
            .retain(|edge| edge.from != component.id && edge.to != component.id);
        true
    }
}

#[derive(Clone, Debug, Default)]
pub struct ValidationReport {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub timing_reliable: bool,
}

pub fn validate_document(doc: &Document) -> ValidationReport {
    let structural = doc.mode == DocumentMode::Structural;
    let flattened = flatten_document(doc);
    let simulation = simulate_flat(&flattened.doc);
    let mut errors = Vec::new();
    let mut warnings = Vec::new();

    for warning in flattened.warnings {
        if structural {
            warnings.push(warning);
        } else {
            errors.push(warning);
        }
    }

    if !structural {
        for node in &flattened.doc.nodes {
            if !matches!(node.kind, NodeKind::Gate) {
                continue;
            }
            if let Some(edge) = flattened.doc.incoming_edge(node.id, 1) {
                if simulation
                    .cell(edge.from)
                    .iter()
                    .any(|step| !matches!(step.value, Value::Bool(_)))
                {
                    errors.push(format!(
                        "Gate '{}' requires a Bool condition cell",
                        node.name
                    ));
                }
            }
        }
    }

    for warning in simulation.warnings {
        if warning.contains("Display-only") {
            if !structural {
                errors.push(warning);
            }
        } else if !structural
            && (warning.contains("input connected")
                || warning.contains("Simultaneous")
                || warning.contains("multiple events"))
        {
            errors.push(warning);
        } else {
            warnings.push(warning);
        }
    }

    let (timing_reliable, timing_warnings) = filter_timing_reliability(&flattened.doc);
    warnings.extend(timing_warnings);

    ValidationReport {
        errors,
        warnings,
        timing_reliable,
    }
}

fn filter_timing_reliability(doc: &Document) -> (bool, Vec<String>) {
    let mut stack: Vec<NodeId> = doc
        .nodes
        .iter()
        .filter(|node| node.display_only)
        .map(|node| node.id)
        .collect();
    if stack.is_empty() {
        return (true, Vec::new());
    }
    let mut seen = HashSet::new();
    let mut warnings = Vec::new();
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        for edge in doc.edges.iter().filter(|edge| edge.from == id) {
            let Some(target) = doc.node(edge.to) else {
                continue;
            };
            if let NodeKind::Filter { op } = &target.kind {
                if !matches!(op, FilterOp::Always) {
                    warnings.push(format!(
                        "Timing event flow may depend on display-only values through filter '{}'",
                        target.name
                    ));
                }
            }
            if matches!(target.kind, NodeKind::Gate) && edge.to_port == 1 {
                warnings.push(format!(
                    "Timing event flow may depend on display-only values through gate '{}'",
                    target.name
                ));
            }
            stack.push(target.id);
        }
    }
    warnings.sort();
    warnings.dedup();
    (warnings.is_empty(), warnings)
}

pub fn simulate(doc: &Document) -> Simulation {
    simulate_with_mapping(doc).0
}

pub fn simulate_with_mapping(doc: &Document) -> (Simulation, FlattenedDocument) {
    let flattened = flatten_document(doc);
    let mut simulation = simulate_flat(&flattened.doc);
    let mut warnings = flattened.warnings.clone();
    warnings.extend(simulation.warnings.clone());
    simulation.warnings = warnings;
    (simulation, flattened)
}

fn simulate_flat(doc: &Document) -> Simulation {
    let end_time = doc.effective_end_time();
    let mut warnings = Vec::new();
    let mut warning_set: HashSet<String> = HashSet::new();
    if doc.mode == DocumentMode::Executable {
        for node in &doc.nodes {
            if node.display_only {
                add_warning(
                    &mut warnings,
                    &mut warning_set,
                    format!(
                        "Display-only node '{}' is not executable; its values are placeholders",
                        node.name
                    ),
                );
            }
        }
    }

    let mut input_events: HashMap<NodeId, HashMap<u64, Value>> = HashMap::new();
    for node in &doc.nodes {
        if let NodeKind::InputEvent { events } = &node.kind {
            let mut map = HashMap::new();
            for ev in events
                .iter()
                .filter(|e| e.time.is_finite() && e.time >= 0.0)
            {
                let key = (ev.time * 1_000_000.0).round() as u64;
                if map.insert(key, ev.value.clone()).is_some() {
                    add_warning(
                        &mut warnings,
                        &mut warning_set,
                        format!(
                            "'{}' has multiple events at t = {}; using the last one",
                            node.name, ev.time
                        ),
                    );
                }
            }
            input_events.insert(node.id, map);
        }
    }

    let mut input_cell_changes: HashMap<NodeId, HashMap<u64, Value>> = HashMap::new();
    for node in &doc.nodes {
        if let NodeKind::InputCell { changes, .. } = &node.kind {
            let mut map = HashMap::new();
            for change in changes
                .iter()
                .filter(|e| e.time.is_finite() && e.time >= 0.0)
            {
                let key = (change.time * 1_000_000.0).round() as u64;
                if map.insert(key, change.value.clone()).is_some() {
                    add_warning(
                        &mut warnings,
                        &mut warning_set,
                        format!(
                            "'{}' has multiple changes at t = {}; using the last one",
                            node.name, change.time
                        ),
                    );
                }
            }
            input_cell_changes.insert(node.id, map);
        }
    }

    let transactions = doc.all_event_times();
    let mut sim = Simulation {
        transactions: transactions.clone(),
        warnings: Vec::new(),
        end_time,
        ..Default::default()
    };

    check_connections(doc, &mut warnings, &mut warning_set);

    let event_order = event_node_order(doc, &mut warnings, &mut warning_set);
    let cell_order = cell_node_order(doc, &mut warnings, &mut warning_set);

    let mut cell_values: HashMap<NodeId, Value> = HashMap::new();
    for &id in &cell_order {
        let node = match doc.node(id) {
            Some(n) => n,
            None => continue,
        };
        let value = match &node.kind {
            NodeKind::InputCell { initial, .. } => initial.clone(),
            NodeKind::Hold { initial } => initial.clone(),
            NodeKind::Map { op, cell: true } => {
                op.apply(&cell_input_value(doc, id, 0, &cell_values))
            }
            NodeKind::Lift { op } => {
                let (a, b) = cell_inputs(doc, id, &cell_values);
                op.apply(&a, &b)
            }
            _ => Value::Unit,
        };
        cell_values.insert(id, value.clone());
        if node.kind.is_cell() {
            sim.cells.insert(id, vec![SimStep { time: 0.0, value }]);
        }
    }

    for &t in &transactions {
        let key = (t * 1_000_000.0).round() as u64;
        let cell_before = cell_values.clone();
        let mut event_values: HashMap<NodeId, Value> = HashMap::new();
        let mut fired_order: Vec<NodeId> = Vec::new();

        for &id in &event_order {
            let node = match doc.node(id) {
                Some(n) => n,
                None => continue,
            };
            let value: Option<Value> = match &node.kind {
                NodeKind::InputEvent { .. } => {
                    input_events.get(&id).and_then(|m| m.get(&key)).cloned()
                }
                NodeKind::Map { op, cell: false } => {
                    stream_input(doc, id, 0, &event_values).map(|v| op.apply(&v))
                }
                NodeKind::Filter { op } => {
                    stream_input(doc, id, 0, &event_values).filter(|v| op.test(v))
                }
                NodeKind::Gate => stream_input(doc, id, 0, &event_values).filter(|_| {
                    matches!(
                        cell_input_value(doc, id, 1, &cell_values),
                        Value::Bool(true)
                    )
                }),
                NodeKind::Merge => {
                    let left = stream_input(doc, id, 0, &event_values);
                    let right = stream_input(doc, id, 1, &event_values);
                    match (left, right) {
                        (Some(_), Some(_)) => {
                            add_warning(
                                &mut warnings,
                                &mut warning_set,
                                format!(
                                    "Simultaneous events on merge '{}' at t = {} (Sodium would reject this); keeping the left event",
                                    node.name, t
                                ),
                            );
                            stream_input(doc, id, 0, &event_values)
                        }
                        (Some(v), None) => Some(v),
                        (None, Some(v)) => Some(v),
                        (None, None) => None,
                    }
                }
                NodeKind::OrElse => {
                    let left = stream_input(doc, id, 0, &event_values);
                    let right = stream_input(doc, id, 1, &event_values);
                    left.or(right)
                }
                NodeKind::Snapshot { op } => {
                    let trigger = stream_input(doc, id, 0, &event_values);
                    trigger.map(|ev| {
                        let cell = cell_input_value(doc, id, 1, &cell_values);
                        op.apply(&ev, &cell)
                    })
                }
                NodeKind::Never => None,
                _ => None,
            };
            if let Some(value) = value {
                event_values.insert(id, value);
                fired_order.push(id);
            }
        }

        for (&id, value) in &event_values {
            if doc.node(id).map(|n| n.kind.is_event()).unwrap_or(false) {
                sim.streams.entry(id).or_default().push(SimEvent {
                    time: t,
                    value: value.clone(),
                });
            }
        }

        let mut updated: HashMap<NodeId, Value> = HashMap::new();
        let mut update_order: Vec<NodeId> = Vec::new();
        for &id in &cell_order {
            let node = match doc.node(id) {
                Some(n) => n,
                None => continue,
            };
            let new_value = match &node.kind {
                NodeKind::InputCell { .. } => input_cell_changes
                    .get(&id)
                    .and_then(|m| m.get(&key))
                    .cloned(),
                NodeKind::Hold { .. } => stream_input(doc, id, 0, &event_values),
                NodeKind::Map { op, cell: true } => doc.incoming_edge(id, 0).map(|edge| {
                    let value = updated
                        .get(&edge.from)
                        .or_else(|| cell_values.get(&edge.from))
                        .unwrap_or(&Value::Unit);
                    op.apply(value)
                }),
                NodeKind::Lift { op } => {
                    let (a, b) = cell_inputs_with_updates(doc, id, &cell_values, &updated);
                    Some(op.apply(&a, &b))
                }
                _ => None,
            };
            if let Some(value) = new_value {
                let old = cell_values.get(&id).cloned().unwrap_or(Value::Unit);
                if value != old {
                    updated.insert(id, value);
                    update_order.push(id);
                }
            }
        }

        for (&id, value) in &updated {
            cell_values.insert(id, value.clone());
            sim.cells.entry(id).or_default().push(SimStep {
                time: t,
                value: value.clone(),
            });
        }

        sim.traces.push(TransactionTrace {
            index: sim.traces.len(),
            time: t,
            events: event_values.clone(),
            event_order: fired_order,
            cell_before,
            cell_after: cell_values.clone(),
            updates: updated.clone(),
            update_order,
        });
    }

    for node in &doc.nodes {
        if node.kind.is_event() {
            let events = sim.streams.entry(node.id).or_default();
            events.sort_by(|a, b| {
                a.time
                    .partial_cmp(&b.time)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        }
        if node.kind.is_cell() {
            sim.cells.entry(node.id).or_default();
        }
    }

    sim.warnings = warnings;
    sim
}

fn add_warning(warnings: &mut Vec<String>, set: &mut HashSet<String>, msg: String) {
    if set.insert(msg.clone()) {
        warnings.push(msg);
    }
}

fn check_connections(
    doc: &Document,
    warnings: &mut Vec<String>,
    warning_set: &mut HashSet<String>,
) {
    for node in &doc.nodes {
        for (port, port_type) in node.kind.input_types().iter().enumerate() {
            if doc.incoming_edge(node.id, port).is_none() {
                let msg = format!(
                    "'{}' has no {} input connected{}",
                    node.name,
                    port_type.name(),
                    if node.kind.input_types().len() > 1 {
                        format!(" (port {})", port + 1)
                    } else {
                        String::new()
                    }
                );
                if warning_set.insert(msg.clone()) {
                    warnings.push(msg);
                }
            }
        }
    }

    for edge in &doc.edges {
        if let (Some(source), Some(target)) = (doc.node(edge.from), doc.node(edge.to)) {
            if let (Some(source_type), Some(target_type)) = (
                source.kind.output_types().get(edge.from_port).copied(),
                target.kind.input_types().get(edge.to_port).copied(),
            ) {
                if !target_type.accepts(source_type) {
                    let msg = format!(
                        "Invalid wire '{}' → '{}': {} output connected to {} input",
                        source.name,
                        target.name,
                        source_type.name(),
                        target_type.name()
                    );
                    if warning_set.insert(msg.clone()) {
                        warnings.push(msg);
                    }
                }
            }
        }
    }
}

fn event_node_order(
    doc: &Document,
    warnings: &mut Vec<String>,
    warning_set: &mut HashSet<String>,
) -> Vec<NodeId> {
    let event_nodes: Vec<NodeId> = doc
        .nodes
        .iter()
        .filter(|n| n.kind.is_event())
        .map(|n| n.id)
        .collect();
    let set: HashSet<NodeId> = event_nodes.iter().copied().collect();
    let mut indegree: HashMap<NodeId, usize> = event_nodes.iter().map(|&id| (id, 0)).collect();
    let mut adj: HashMap<NodeId, Vec<NodeId>> = HashMap::new();

    for edge in &doc.edges {
        if !set.contains(&edge.from) || !set.contains(&edge.to) {
            continue;
        }
        if edge.to_port
            >= doc
                .node(edge.to)
                .map(|n| n.kind.input_types().len())
                .unwrap_or(0)
        {
            continue;
        }
        if doc
            .node(edge.to)
            .map(|n| n.kind.input_types()[edge.to_port] == crate::model::PortType::Cell)
            .unwrap_or(false)
        {
            continue;
        }
        adj.entry(edge.from).or_default().push(edge.to);
        *indegree.entry(edge.to).or_default() += 1;
    }

    let mut queue: Vec<NodeId> = event_nodes
        .iter()
        .copied()
        .filter(|id| indegree.get(id).copied().unwrap_or(0) == 0)
        .collect();
    queue.sort_unstable();
    let mut order = Vec::new();
    let mut qidx = 0;
    while qidx < queue.len() {
        let id = queue[qidx];
        qidx += 1;
        order.push(id);
        if let Some(next) = adj.get(&id) {
            let mut ready = Vec::new();
            for &to in next {
                if let Some(deg) = indegree.get_mut(&to) {
                    *deg = deg.saturating_sub(1);
                    if *deg == 0 {
                        ready.push(to);
                    }
                }
            }
            ready.sort_unstable();
            queue.extend(ready);
        }
    }

    if order.len() != event_nodes.len() {
        let mut unresolved: Vec<NodeId> = event_nodes
            .iter()
            .copied()
            .filter(|id| !order.contains(id))
            .collect();
        unresolved.sort_unstable();
        for id in &unresolved {
            if let Some(node) = doc.node(*id) {
                let msg = format!(
                    "Event cycle detected at '{}'; it is not evaluated (use hold to break feedback)",
                    node.name
                );
                if warning_set.insert(msg.clone()) {
                    warnings.push(msg);
                }
            }
        }
        order.extend(unresolved);
    }
    order
}

fn cell_node_order(
    doc: &Document,
    warnings: &mut Vec<String>,
    warning_set: &mut HashSet<String>,
) -> Vec<NodeId> {
    let cell_nodes: Vec<NodeId> = doc
        .nodes
        .iter()
        .filter(|n| n.kind.is_cell())
        .map(|n| n.id)
        .collect();
    let set: HashSet<NodeId> = cell_nodes.iter().copied().collect();
    let mut indegree: HashMap<NodeId, usize> = cell_nodes.iter().map(|&id| (id, 0)).collect();
    let mut adj: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    for edge in &doc.edges {
        if set.contains(&edge.from) && set.contains(&edge.to) {
            adj.entry(edge.from).or_default().push(edge.to);
            *indegree.entry(edge.to).or_default() += 1;
        }
    }
    let mut queue: Vec<NodeId> = cell_nodes
        .iter()
        .copied()
        .filter(|id| indegree.get(id).copied().unwrap_or(0) == 0)
        .collect();
    queue.sort_unstable();
    let mut order = Vec::new();
    let mut qidx = 0;
    while qidx < queue.len() {
        let id = queue[qidx];
        qidx += 1;
        order.push(id);
        if let Some(next) = adj.get(&id) {
            let mut ready = Vec::new();
            for &to in next {
                if let Some(deg) = indegree.get_mut(&to) {
                    *deg = deg.saturating_sub(1);
                    if *deg == 0 {
                        ready.push(to);
                    }
                }
            }
            ready.sort_unstable();
            queue.extend(ready);
        }
    }
    if order.len() != cell_nodes.len() {
        let mut unresolved: Vec<NodeId> = cell_nodes
            .iter()
            .copied()
            .filter(|id| !order.contains(id))
            .collect();
        unresolved.sort_unstable();
        for id in &unresolved {
            if let Some(node) = doc.node(*id) {
                let msg = format!(
                    "Cell loop detected at '{}'; it is evaluated with its previous value",
                    node.name
                );
                if warning_set.insert(msg.clone()) {
                    warnings.push(msg);
                }
            }
        }
        order.extend(unresolved);
    }
    order
}

fn stream_input(
    doc: &Document,
    node: NodeId,
    port: usize,
    event_values: &HashMap<NodeId, Value>,
) -> Option<Value> {
    let edge = doc.incoming_edge(node, port)?;
    event_values.get(&edge.from).cloned()
}

fn cell_input_value(
    doc: &Document,
    node: NodeId,
    port: usize,
    cell_values: &HashMap<NodeId, Value>,
) -> Value {
    let Some(edge) = doc.incoming_edge(node, port) else {
        return Value::Unit;
    };
    cell_values.get(&edge.from).cloned().unwrap_or(Value::Unit)
}

fn cell_inputs(
    doc: &Document,
    node: NodeId,
    cell_values: &HashMap<NodeId, Value>,
) -> (Value, Value) {
    (
        cell_input_value(doc, node, 0, cell_values),
        cell_input_value(doc, node, 1, cell_values),
    )
}

fn cell_inputs_with_updates(
    doc: &Document,
    node: NodeId,
    cell_values: &HashMap<NodeId, Value>,
    updated: &HashMap<NodeId, Value>,
) -> (Value, Value) {
    let get = |port: usize| -> Value {
        let Some(edge) = doc.incoming_edge(node, port) else {
            return Value::Unit;
        };
        updated
            .get(&edge.from)
            .or_else(|| cell_values.get(&edge.from))
            .cloned()
            .unwrap_or(Value::Unit)
    };
    (get(0), get(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        CombineOp, ComponentDef, FilterOp, NodeKind, PortSpec, PortType, TimedValue, UnaryOp, Value,
    };

    fn accum_doc() -> Document {
        let mut doc = Document::blank();
        let delta = doc.add_node(
            NodeKind::InputEvent {
                events: vec![
                    TimedValue::new(1.0, Value::Number(5.0)),
                    TimedValue::new(2.0, Value::Number(10.0)),
                    TimedValue::new(3.0, Value::Number(-3.0)),
                ],
            },
            [0.0, 0.0],
        );
        let snap = doc.add_node(NodeKind::Snapshot { op: CombineOp::Add }, [220.0, 0.0]);
        let hold = doc.add_node(
            NodeKind::Hold {
                initial: Value::Number(0.0),
            },
            [220.0, 160.0],
        );
        let sink = doc.add_node(NodeKind::Sink, [440.0, 0.0]);
        doc.add_edge(delta, snap, 0).unwrap();
        doc.add_edge(hold, snap, 1).unwrap();
        doc.add_edge(snap, hold, 0).unwrap();
        doc.add_edge(snap, sink, 0).unwrap();
        if let Some(node) = doc.node_mut(delta) {
            node.name = "sDelta".into();
        }
        if let Some(node) = doc.node_mut(snap) {
            node.name = "total".into();
        }
        if let Some(node) = doc.node_mut(hold) {
            node.name = "accum".into();
        }
        doc
    }

    #[test]
    fn accumulator_matches_sodium_semantics() {
        let doc = accum_doc();
        let sim = simulate(&doc);
        let snap = doc.nodes.iter().find(|n| n.name == "total").unwrap().id;
        let hold = doc.nodes.iter().find(|n| n.name == "accum").unwrap().id;
        let events: Vec<(f64, String)> = sim
            .stream(snap)
            .iter()
            .map(|e| (e.time, e.value.to_string()))
            .collect();
        assert_eq!(
            events,
            vec![
                (1.0, "5".to_string()),
                (2.0, "15".to_string()),
                (3.0, "12".to_string())
            ]
        );
        let steps: Vec<(f64, String)> = sim
            .cell(hold)
            .iter()
            .map(|s| (s.time, s.value.to_string()))
            .collect();
        assert_eq!(
            steps,
            vec![
                (0.0, "0".to_string()),
                (1.0, "5".to_string()),
                (2.0, "15".to_string()),
                (3.0, "12".to_string())
            ]
        );
    }

    #[test]
    fn snapshot_sees_the_value_before_the_transaction() {
        let mut doc = Document::blank();
        let trigger = doc.add_node(
            NodeKind::InputEvent {
                events: vec![TimedValue::new(1.0, Value::Unit)],
            },
            [0.0, 0.0],
        );
        let text = doc.add_node(
            NodeKind::InputCell {
                initial: Value::Text("hello".into()),
                changes: vec![TimedValue::new(1.0, Value::Text("world".into()))],
            },
            [0.0, 140.0],
        );
        let snap = doc.add_node(
            NodeKind::Snapshot {
                op: CombineOp::KeepRight,
            },
            [220.0, 60.0],
        );
        doc.add_edge(trigger, snap, 0).unwrap();
        doc.add_edge(text, snap, 1).unwrap();
        let sim = simulate(&doc);
        assert_eq!(sim.stream(snap).len(), 1);
        assert_eq!(sim.stream(snap)[0].value.to_string(), "hello");
    }

    #[test]
    fn constant_map_replaces_the_value() {
        let mut doc = Document::blank();
        let source = doc.add_node(
            NodeKind::InputEvent {
                events: vec![TimedValue::new(1.0, Value::Number(42.0))],
            },
            [0.0, 0.0],
        );
        let map = doc.add_node(
            NodeKind::Map {
                op: UnaryOp::Constant(0.0),
                cell: false,
            },
            [200.0, 0.0],
        );
        doc.add_edge(source, map, 0).unwrap();
        let sim = simulate(&doc);
        assert_eq!(sim.stream(map)[0].value.to_string(), "0");
    }

    #[test]
    fn map_filter_and_merge() {
        let mut doc = Document::blank();
        let left = doc.add_node(
            NodeKind::InputEvent {
                events: vec![
                    TimedValue::new(1.0, Value::Number(1.0)),
                    TimedValue::new(3.0, Value::Number(3.0)),
                ],
            },
            [0.0, 0.0],
        );
        let right = doc.add_node(
            NodeKind::InputEvent {
                events: vec![TimedValue::new(2.0, Value::Number(-20.0))],
            },
            [0.0, 140.0],
        );
        let merge = doc.add_node(NodeKind::Merge, [200.0, 60.0]);
        let map = doc.add_node(
            NodeKind::Map {
                op: UnaryOp::Add(10.0),
                cell: false,
            },
            [400.0, 60.0],
        );
        let filter = doc.add_node(
            NodeKind::Filter {
                op: crate::model::FilterOp::Positive,
            },
            [600.0, 60.0],
        );
        doc.add_edge(left, merge, 0).unwrap();
        doc.add_edge(right, merge, 1).unwrap();
        doc.add_edge(merge, map, 0).unwrap();
        doc.add_edge(map, filter, 0).unwrap();
        let sim = simulate(&doc);
        let values: Vec<String> = sim
            .stream(filter)
            .iter()
            .map(|e| e.value.to_string())
            .collect();
        assert_eq!(values, vec!["11", "13"]);
    }

    #[test]
    fn lift_computes_initial_and_updates() {
        let mut doc = Document::blank();
        let a = doc.add_node(
            NodeKind::InputCell {
                initial: Value::Number(1.0),
                changes: vec![TimedValue::new(1.0, Value::Number(5.0))],
            },
            [0.0, 0.0],
        );
        let b = doc.add_node(
            NodeKind::InputCell {
                initial: Value::Number(2.0),
                changes: vec![],
            },
            [0.0, 140.0],
        );
        let lift = doc.add_node(NodeKind::Lift { op: CombineOp::Add }, [220.0, 60.0]);
        doc.add_edge(a, lift, 0).unwrap();
        doc.add_edge(b, lift, 1).unwrap();
        let sim = simulate(&doc);
        let steps: Vec<String> = sim.cell(lift).iter().map(|s| s.value.to_string()).collect();
        assert_eq!(steps, vec!["3", "7"]);
    }

    #[test]
    fn calibration_example_clears_and_scales() {
        let doc = crate::examples::load_example("calibration").unwrap();
        let sim = simulate(&doc);
        assert!(sim.warnings.is_empty(), "{:?}", sim.warnings);
        let find = |name: &str| doc.nodes.iter().find(|n| n.name == name).unwrap().id;
        let hold: Vec<String> = sim
            .cell(find("hold"))
            .iter()
            .map(|s| s.value.to_string())
            .collect();
        assert_eq!(hold, vec!["0", "5", "15", "0", "-3", "5"]);
        let snapshot: Vec<String> = sim
            .stream(find("snapshot"))
            .iter()
            .map(|e| e.value.to_string())
            .collect();
        assert_eq!(snapshot, vec!["5", "15", "-3", "5"]);
        let lift: Vec<String> = sim
            .cell(find("lift"))
            .iter()
            .map(|s| s.value.to_string())
            .collect();
        assert_eq!(lift, vec!["0", "5", "15", "0", "-6", "10"]);
    }

    #[test]
    fn component_is_flattened_and_simulated() {
        let mut internal = Document::blank();
        let input = internal.add_node(NodeKind::InputEvent { events: vec![] }, [0.0, 0.0]);
        let doubled = internal.add_node(
            NodeKind::Map {
                op: UnaryOp::Multiply(2.0),
                cell: false,
            },
            [200.0, 0.0],
        );
        let out = internal.add_node(NodeKind::Sink, [400.0, 0.0]);
        internal.add_edge(input, doubled, 0).unwrap();
        internal.add_edge(doubled, out, 0).unwrap();
        internal.node_mut(input).unwrap().name = "in".into();
        internal.node_mut(out).unwrap().name = "out".into();
        internal.node_mut(doubled).unwrap().name = "timesTwo".into();

        let inputs = vec![PortSpec {
            name: "in".into(),
            ty: PortType::Stream,
        }];
        let outputs = vec![PortSpec {
            name: "out".into(),
            ty: PortType::Stream,
        }];

        let mut doc = Document::blank();
        doc.components.push(ComponentDef {
            name: "double".into(),
            inputs: inputs.clone(),
            outputs: outputs.clone(),
            body: Vec::new(),
            doc: Box::new(internal),
        });
        let source = doc.add_node(
            NodeKind::InputEvent {
                events: vec![
                    TimedValue::new(1.0, Value::Number(1.0)),
                    TimedValue::new(2.0, Value::Number(2.0)),
                    TimedValue::new(3.0, Value::Number(3.0)),
                ],
            },
            [0.0, 0.0],
        );
        let component = doc.add_node(
            NodeKind::Component {
                definition: Some("double".into()),
                inputs,
                outputs,
                body: Vec::new(),
            },
            [220.0, 0.0],
        );
        let observed = doc.add_node(
            NodeKind::Map {
                op: UnaryOp::Identity,
                cell: false,
            },
            [440.0, 0.0],
        );
        let sink = doc.add_node(NodeKind::Sink, [660.0, 0.0]);
        doc.add_edge_from_port(source, 0, component, 0).unwrap();
        doc.add_edge_from_port(component, 0, observed, 0).unwrap();
        doc.add_edge(observed, sink, 0).unwrap();
        doc.node_mut(observed).unwrap().name = "observed".into();

        let (flat, warnings) = flatten_components(&doc);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(flat.nodes.iter().all(|node| !node.kind.is_component()));

        let sim = simulate(&doc);
        assert!(sim.warnings.is_empty(), "{:?}", sim.warnings);
        let values: Vec<String> = sim
            .stream(observed)
            .iter()
            .map(|event| event.value.to_string())
            .collect();
        assert_eq!(values, vec!["2", "4", "6"]);
    }

    #[test]
    fn accumulate_component_matches_the_calibration_values() {
        let doc = crate::examples::load_example("component").unwrap();
        let sim = simulate(&doc);
        assert!(sim.warnings.is_empty(), "{:?}", sim.warnings);
        let observed = doc
            .nodes
            .iter()
            .find(|node| node.name == "observed")
            .unwrap()
            .id;
        let values: Vec<String> = sim
            .stream(observed)
            .iter()
            .map(|event| event.value.to_string())
            .collect();
        assert_eq!(values, vec!["5", "15", "0", "-3", "5"]);
    }

    #[test]
    fn component_supports_multiple_outputs() {
        let mut internal = Document::blank();
        let input = internal.add_node(NodeKind::InputEvent { events: vec![] }, [0.0, 0.0]);
        let copied = internal.add_node(
            NodeKind::Map {
                op: UnaryOp::Identity,
                cell: false,
            },
            [200.0, 0.0],
        );
        let out_a = internal.add_node(NodeKind::Sink, [400.0, 0.0]);
        let out_b = internal.add_node(NodeKind::Sink, [400.0, 140.0]);
        internal.add_edge(input, copied, 0).unwrap();
        internal.add_edge(copied, out_a, 0).unwrap();
        internal.add_edge(copied, out_b, 0).unwrap();
        internal.node_mut(input).unwrap().name = "in".into();
        internal.node_mut(out_a).unwrap().name = "a".into();
        internal.node_mut(out_b).unwrap().name = "b".into();

        let inputs = vec![PortSpec {
            name: "in".into(),
            ty: PortType::Stream,
        }];
        let outputs = vec![
            PortSpec {
                name: "a".into(),
                ty: PortType::Stream,
            },
            PortSpec {
                name: "b".into(),
                ty: PortType::Stream,
            },
        ];

        let mut doc = Document::blank();
        doc.components.push(ComponentDef {
            name: "split".into(),
            inputs: inputs.clone(),
            outputs: outputs.clone(),
            body: Vec::new(),
            doc: Box::new(internal),
        });
        let source = doc.add_node(
            NodeKind::InputEvent {
                events: vec![
                    TimedValue::new(1.0, Value::Number(1.0)),
                    TimedValue::new(2.0, Value::Number(2.0)),
                ],
            },
            [0.0, 0.0],
        );
        let component = doc.add_node(
            NodeKind::Component {
                definition: Some("split".into()),
                inputs,
                outputs,
                body: Vec::new(),
            },
            [220.0, 0.0],
        );
        let observed_a = doc.add_node(
            NodeKind::Map {
                op: UnaryOp::Identity,
                cell: false,
            },
            [440.0, 0.0],
        );
        let observed_b = doc.add_node(
            NodeKind::Map {
                op: UnaryOp::Identity,
                cell: false,
            },
            [440.0, 140.0],
        );
        doc.add_edge_from_port(source, 0, component, 0).unwrap();
        doc.add_edge_from_port(component, 0, observed_a, 0).unwrap();
        doc.add_edge_from_port(component, 1, observed_b, 0).unwrap();

        let sim = simulate(&doc);
        assert!(sim.warnings.is_empty(), "{:?}", sim.warnings);
        for id in [observed_a, observed_b] {
            let values: Vec<String> = sim
                .stream(id)
                .iter()
                .map(|event| event.value.to_string())
                .collect();
            assert_eq!(values, vec!["1", "2"]);
        }
    }

    #[test]
    fn validation_reports_missing_required_inputs() {
        let mut doc = Document::blank();
        doc.add_node(
            NodeKind::Map {
                op: UnaryOp::Identity,
                cell: false,
            },
            [0.0, 0.0],
        );
        let report = validate_document(&doc);
        assert!(report
            .errors
            .iter()
            .any(|error| error.contains("input connected")));
    }

    #[test]
    fn validation_accepts_all_examples() {
        for (_, key) in crate::examples::EXAMPLES {
            let doc = crate::examples::load_example(key).unwrap();
            let report = validate_document(&doc);
            assert!(report.errors.is_empty(), "{key}: {:?}", report.errors);
        }
    }

    #[test]
    fn display_only_values_make_filter_timing_unreliable() {
        let mut doc = Document::blank();
        doc.mode = DocumentMode::Structural;
        let source = doc.add_node(
            NodeKind::InputEvent {
                events: vec![TimedValue::new(1.0, Value::Number(1.0))],
            },
            [0.0, 0.0],
        );
        let display = doc.add_node(
            NodeKind::Map {
                op: UnaryOp::Identity,
                cell: false,
            },
            [200.0, 0.0],
        );
        if let Some(node) = doc.node_mut(display) {
            node.display_only = true;
            node.label = "opaque".into();
        }
        let filter = doc.add_node(
            NodeKind::Filter {
                op: FilterOp::Positive,
            },
            [400.0, 0.0],
        );
        let sink = doc.add_node(NodeKind::Sink, [600.0, 0.0]);
        doc.add_edge(source, display, 0).unwrap();
        doc.add_edge(display, filter, 0).unwrap();
        doc.add_edge(filter, sink, 0).unwrap();
        let report = validate_document(&doc);
        assert!(!report.timing_reliable);
        assert!(report
            .warnings
            .iter()
            .any(|warning| warning.contains("display-only values")));
    }

    #[test]
    fn validation_treats_simultaneous_events_as_errors() {
        let mut doc = Document::blank();
        let a = doc.add_node(
            NodeKind::InputEvent {
                events: vec![TimedValue::new(1.0, Value::Number(1.0))],
            },
            [0.0, 0.0],
        );
        let b = doc.add_node(
            NodeKind::InputEvent {
                events: vec![TimedValue::new(1.0, Value::Number(2.0))],
            },
            [0.0, 100.0],
        );
        let merge = doc.add_node(NodeKind::Merge, [200.0, 50.0]);
        doc.add_edge(a, merge, 0).unwrap();
        doc.add_edge(b, merge, 1).unwrap();
        let report = validate_document(&doc);
        assert!(report
            .errors
            .iter()
            .any(|error| error.contains("Simultaneous")));
    }

    #[test]
    fn simultaneous_merge_is_reported() {
        let mut doc = Document::blank();
        let a = doc.add_node(
            NodeKind::InputEvent {
                events: vec![TimedValue::new(1.0, Value::Number(1.0))],
            },
            [0.0, 0.0],
        );
        let b = doc.add_node(
            NodeKind::InputEvent {
                events: vec![TimedValue::new(1.0, Value::Number(2.0))],
            },
            [0.0, 140.0],
        );
        let merge = doc.add_node(NodeKind::Merge, [200.0, 60.0]);
        doc.add_edge(a, merge, 0).unwrap();
        doc.add_edge(b, merge, 1).unwrap();
        let sim = simulate(&doc);
        assert!(sim.warnings.iter().any(|w| w.contains("Simultaneous")));
        assert_eq!(sim.stream(merge)[0].value.to_string(), "1");
    }

    #[test]
    fn or_else_prefers_left_without_warning() {
        let mut doc = Document::blank();
        let a = doc.add_node(
            NodeKind::InputEvent {
                events: vec![
                    TimedValue::new(1.0, Value::Number(1.0)),
                    TimedValue::new(3.0, Value::Number(3.0)),
                ],
            },
            [0.0, 0.0],
        );
        let b = doc.add_node(
            NodeKind::InputEvent {
                events: vec![
                    TimedValue::new(1.0, Value::Number(2.0)),
                    TimedValue::new(2.0, Value::Number(20.0)),
                ],
            },
            [0.0, 140.0],
        );
        let or_else = doc.add_node(NodeKind::OrElse, [200.0, 60.0]);
        doc.add_edge(a, or_else, 0).unwrap();
        doc.add_edge(b, or_else, 1).unwrap();
        let sim = simulate(&doc);
        assert!(!sim.warnings.iter().any(|w| w.contains("Simultaneous")));
        let values: Vec<String> = sim
            .stream(or_else)
            .iter()
            .map(|event| event.value.to_string())
            .collect();
        assert_eq!(values, vec!["1", "20", "3"]);

        let report = validate_document(&doc);
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    }
}
