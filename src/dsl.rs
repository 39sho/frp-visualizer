use crate::layout::auto_layout_tree;
use crate::model::{
    Annotation, AnnotationTarget, CombineOp, ComponentDef, Document, FilterOp, NodeId, NodeKind,
    PortSpec, PortType, TimedValue, UnaryOp, Value,
};

#[derive(Debug)]
pub struct ParsedDsl {
    pub doc: Document,
    pub auto_layout: bool,
}

pub fn parse_file(path: &std::path::Path) -> Result<ParsedDsl, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    parse(&text)
}

pub fn parse(input: &str) -> Result<ParsedDsl, String> {
    let mut parser = Parser::new(input);
    let mut doc = Document::blank();
    parser.parse_top(&mut doc)?;
    doc.components = parser.definitions;
    if parser.auto_layout {
        auto_layout_tree(&mut doc);
    }
    Ok(ParsedDsl {
        doc,
        auto_layout: parser.auto_layout,
    })
}

struct Parser {
    lines: Vec<(usize, String)>,
    index: usize,
    definitions: Vec<ComponentDef>,
    auto_layout: bool,
}

impl Parser {
    fn new(input: &str) -> Self {
        let lines = input
            .lines()
            .enumerate()
            .map(|(index, line)| (index + 1, line.trim().to_string()))
            .filter(|(_, line)| !line.is_empty() && !line.starts_with('#'))
            .collect();
        Self {
            lines,
            index: 0,
            definitions: Vec::new(),
            auto_layout: false,
        }
    }

    fn next_line(&mut self) -> Option<(usize, String)> {
        let line = self.lines.get(self.index).cloned();
        if line.is_some() {
            self.index += 1;
        }
        line
    }

    fn parse_top(&mut self, doc: &mut Document) -> Result<(), String> {
        while let Some((line_number, line)) = self.next_line() {
            if line.starts_with("component ") && line.ends_with('{') {
                self.parse_component_definition(line_number, &line)?;
            } else {
                self.parse_statement(doc, line_number, &line)?;
            }
        }
        Ok(())
    }

    fn parse_statement(
        &mut self,
        doc: &mut Document,
        line_number: usize,
        line: &str,
    ) -> Result<(), String> {
        self.parse_statement_inner(doc, line_number, line)
            .map_err(|error| format!("line {line_number}: {error}"))
    }

    fn parse_statement_inner(
        &mut self,
        doc: &mut Document,
        _line_number: usize,
        line: &str,
    ) -> Result<(), String> {
        if let Some(rest) = line.strip_prefix("note ") {
            return self.parse_note(doc, rest);
        }
        if let Some(rest) = line.strip_prefix("diagram ") {
            let title = rest.trim();
            if title.is_empty() {
                return Err("diagram title is empty".into());
            }
            doc.title = title.to_string();
            return Ok(());
        }
        if let Some(rest) = line.strip_prefix("layout ") {
            match rest.trim() {
                "auto" => self.auto_layout = true,
                "manual" => self.auto_layout = false,
                other => return Err(format!("unknown layout '{other}' (use auto or manual)")),
            }
            return Ok(());
        }
        if let Some(rest) = line.strip_prefix("time_end ") {
            doc.time_end = rest
                .trim()
                .parse::<f64>()
                .map_err(|_| "time_end must be a number".to_string())?;
            return Ok(());
        }
        if let Some(rest) = line.strip_prefix("mode ") {
            doc.mode = match rest.trim() {
                "executable" => crate::model::DocumentMode::Executable,
                "structural" => crate::model::DocumentMode::Structural,
                other => {
                    return Err(format!(
                        "unknown mode '{other}' (use executable or structural)"
                    ));
                }
            };
            return Ok(());
        }
        if line.starts_with("component ") && line.contains('=') {
            return self.parse_component_instance(doc, line);
        }
        if line.contains("->") {
            return self.parse_edge(doc, line);
        }
        self.parse_node(doc, line)
    }

    fn parse_node(&mut self, doc: &mut Document, line: &str) -> Result<(), String> {
        let (line, position) = split_at_suffix(line);
        let Some((keyword, rest)) = line.split_once(char::is_whitespace) else {
            return Err(format!("unknown statement '{line}'"));
        };
        let rest = rest.trim();
        let (name, tail) = split_name(rest)?;
        ensure_unique_name(doc, name)?;
        let position = position.unwrap_or_else(|| default_position(doc));
        let mut display_label = None;
        let mut display_only = false;

        let kind = match keyword {
            "stream" => {
                let events = if tail.is_empty() {
                    Vec::new()
                } else {
                    let events = tail
                        .strip_prefix("events")
                        .ok_or("expected 'events <time:value>, ...'")?
                        .trim();
                    parse_timed_values(events)?
                };
                NodeKind::InputEvent { events }
            }
            "cell" => {
                let Some(value_text) = tail.strip_prefix('=') else {
                    return Err("expected 'cell <name> = <initial> [changes ...]'".into());
                };
                let (initial, changes) = match value_text.split_once(" changes ") {
                    Some((initial, changes)) => (initial, Some(changes)),
                    None => (value_text, None),
                };
                let changes = match changes {
                    Some(changes) => parse_timed_values(changes.trim())?,
                    None => Vec::new(),
                };
                NodeKind::InputCell {
                    initial: parse_value(initial.trim()),
                    changes,
                }
            }
            "map" => {
                let op = tail
                    .strip_prefix('=')
                    .ok_or("expected 'map <name> = <op>'")?
                    .trim();
                let op = if let Some(display) = op.strip_prefix("display ") {
                    let (text, rest) = take_quoted(display)?;
                    if !rest.trim().is_empty() {
                        return Err("unexpected text after display label".into());
                    }
                    display_label = Some(text);
                    display_only = true;
                    UnaryOp::Identity
                } else {
                    parse_unary_op(op)?
                };
                NodeKind::Map { op }
            }
            "filter" => {
                let op = tail
                    .strip_prefix('=')
                    .ok_or("expected 'filter <name> = <op>'")?
                    .trim();
                let op = if let Some(display) = op.strip_prefix("display ") {
                    let (text, rest) = take_quoted(display)?;
                    if !rest.trim().is_empty() {
                        return Err("unexpected text after display label".into());
                    }
                    display_label = Some(text);
                    display_only = true;
                    FilterOp::Always
                } else {
                    parse_filter_op(op)?
                };
                NodeKind::Filter { op }
            }
            "snapshot" => {
                let op = tail
                    .strip_prefix('=')
                    .ok_or("expected 'snapshot <name> = <op>'")?
                    .trim();
                NodeKind::Snapshot {
                    op: parse_combine_op(op)?,
                }
            }
            "lift" => {
                let op = tail
                    .strip_prefix('=')
                    .ok_or("expected 'lift <name> = <op>'")?
                    .trim();
                NodeKind::Lift {
                    op: parse_combine_op(op)?,
                }
            }
            "hold" => {
                let value = tail
                    .strip_prefix('=')
                    .ok_or("expected 'hold <name> = <value>'")?
                    .trim();
                NodeKind::Hold {
                    initial: parse_value(value),
                }
            }
            "merge" => {
                if !tail.is_empty() {
                    return Err("merge does not take a value".into());
                }
                NodeKind::Merge
            }
            "never" => {
                if !tail.is_empty() {
                    return Err("never does not take a value".into());
                }
                NodeKind::Never
            }
            "sink" => {
                if !tail.is_empty() {
                    return Err("sink does not take a value".into());
                }
                NodeKind::Sink
            }
            other => return Err(format!("unknown node type '{other}'")),
        };

        let id = doc.add_node(kind, position);
        if let Some(node) = doc.node_mut(id) {
            node.name = name.to_string();
            if let Some(label) = display_label {
                node.label = label;
                node.display_only = display_only;
            }
        }
        Ok(())
    }

    fn parse_component_instance(&mut self, doc: &mut Document, line: &str) -> Result<(), String> {
        let (line, position) = split_at_suffix(line);
        let rest = line
            .strip_prefix("component ")
            .ok_or("expected 'component <instance> = <definition>'")?;
        let (instance, definition) = rest
            .split_once('=')
            .ok_or("expected 'component <instance> = <definition>'")?;
        let instance = instance.trim();
        let definition_name = definition.trim();
        validate_name(instance)?;
        ensure_unique_name(doc, instance)?;
        let Some(component) = self
            .definitions
            .iter()
            .find(|candidate| candidate.name == definition_name)
        else {
            return Err(format!(
                "unknown component definition '{definition_name}' (define it before use)"
            ));
        };
        let kind = NodeKind::Component {
            definition: Some(component.name.clone()),
            inputs: component.inputs.clone(),
            outputs: component.outputs.clone(),
            body: component.body.clone(),
        };
        let id = doc.add_node(kind, position.unwrap_or_else(|| default_position(doc)));
        if let Some(node) = doc.node_mut(id) {
            node.name = instance.to_string();
        }
        Ok(())
    }

    fn parse_edge(&mut self, doc: &mut Document, line: &str) -> Result<(), String> {
        let (edge_text, label) = match line.rsplit_once(" : ") {
            Some((edge, label)) => (edge.trim(), label.trim()),
            None => (line, ""),
        };
        let (label, label_width) = match label.rsplit_once(" @") {
            Some((label, width)) => {
                let width = width
                    .trim()
                    .parse::<f32>()
                    .map_err(|_| "edge label width must be a number".to_string())?;
                (label.trim(), Some(width))
            }
            None => (label, None),
        };
        let (from, to) = edge_text
            .split_once("->")
            .ok_or("expected '<from> -> <to>'")?;
        let (from, from_port) = resolve_port(doc, from.trim(), false)?;
        let (to, to_port) = resolve_port(doc, to.trim(), true)?;
        let edge_id = doc
            .add_edge_from_port(from, from_port, to, to_port)
            .map_err(|error| error.to_string())?;
        if !label.is_empty() || label_width.is_some() {
            if let Some(edge) = doc.edges.iter_mut().find(|edge| edge.id == edge_id) {
                edge.label = label.replace("\\n", "\n");
                edge.label_width = label_width;
            }
        }
        Ok(())
    }

    fn parse_note(&mut self, doc: &mut Document, rest: &str) -> Result<(), String> {
        let (text, rest) = take_quoted(rest)?;
        let rest = rest.trim_start();
        let rest = rest
            .strip_prefix("on ")
            .ok_or("expected 'note \"<text>\" on <node> [at x,y]'")?;
        let (target, position) = split_at_suffix(rest);
        let target = if let Some(index) = target.strip_prefix("tx ") {
            let index = index
                .trim()
                .parse::<usize>()
                .map_err(|_| "transaction number must be an integer".to_string())?;
            if index == 0 {
                return Err("transaction numbers are 1-based".into());
            }
            AnnotationTarget::Transaction { index: index - 1 }
        } else {
            let id = find_node(doc, target.trim())
                .ok_or_else(|| format!("unknown note target '{}'", target.trim()))?;
            AnnotationTarget::Node { id }
        };
        doc.annotations.push(Annotation {
            text,
            target: Some(target),
            position,
        });
        Ok(())
    }

    fn parse_component_definition(
        &mut self,
        line_number: usize,
        header: &str,
    ) -> Result<(), String> {
        let name = header
            .strip_prefix("component ")
            .and_then(|rest| rest.strip_suffix('{'))
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .ok_or("expected 'component <name> {'")?;
        validate_name(name)?;
        if self
            .definitions
            .iter()
            .any(|definition| definition.name == name)
        {
            return Err(format!("duplicate component definition '{name}'"));
        }

        let mut inputs: Vec<PortSpec> = Vec::new();
        let mut outputs: Vec<PortSpec> = Vec::new();
        let mut body = Vec::new();
        let mut statements = Vec::new();
        loop {
            let Some((line_number, line)) = self.next_line() else {
                return Err(format!(
                    "line {line_number}: unclosed component definition '{name}'"
                ));
            };
            if line == "}" {
                break;
            }
            if let Some(rest) = line.strip_prefix("in ") {
                let port = parse_port(rest, "input")?;
                if inputs.iter().any(|existing| existing.name == port.name)
                    || outputs.iter().any(|existing| existing.name == port.name)
                {
                    return Err(format!(
                        "line {line_number}: duplicate component port '{}'",
                        port.name
                    ));
                }
                inputs.push(port);
            } else if let Some(rest) = line.strip_prefix("out ") {
                let port = parse_port(rest, "output")?;
                if inputs.iter().any(|existing| existing.name == port.name)
                    || outputs.iter().any(|existing| existing.name == port.name)
                {
                    return Err(format!(
                        "line {line_number}: duplicate component port '{}'",
                        port.name
                    ));
                }
                outputs.push(port);
            } else if let Some(rest) = line.strip_prefix("body ") {
                let (text, rest) = take_quoted(rest)?;
                if !rest.trim().is_empty() {
                    return Err("unexpected text after body string".into());
                }
                body.push(text);
            } else if line.starts_with("component ") && line.ends_with('{') {
                self.parse_component_definition(line_number, &line)?;
            } else {
                statements.push((line_number, line));
            }
        }

        let mut internal = Document::blank();
        internal.title = name.to_string();
        for (index, port) in inputs.iter().enumerate() {
            let kind = match port.ty {
                PortType::Stream => NodeKind::InputEvent { events: Vec::new() },
                PortType::Cell => NodeKind::InputCell {
                    initial: Value::Unit,
                    changes: Vec::new(),
                },
                PortType::Any => {
                    return Err(format!(
                        "component input '{}' cannot have type 'any'",
                        port.name
                    ));
                }
            };
            let id = internal.add_node(kind, [40.0, 80.0 + index as f32 * 120.0]);
            if let Some(node) = internal.node_mut(id) {
                node.name = port.name.clone();
            }
        }
        for (index, port) in outputs.iter().enumerate() {
            let id = internal.add_node(NodeKind::Sink, [900.0, 80.0 + index as f32 * 120.0]);
            if let Some(node) = internal.node_mut(id) {
                node.name = port.name.clone();
            }
        }
        for (line_number, line) in statements {
            self.parse_statement(&mut internal, line_number, &line)?;
        }

        self.definitions.push(ComponentDef {
            name: name.to_string(),
            inputs,
            outputs,
            body,
            doc: Box::new(internal),
        });
        Ok(())
    }
}

fn split_name(rest: &str) -> Result<(&str, &str), String> {
    let rest = rest.trim();
    let Some((name, tail)) = rest.split_once(char::is_whitespace) else {
        validate_name(rest)?;
        return Ok((rest, ""));
    };
    validate_name(name)?;
    Ok((name, tail.trim()))
}

fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("name is empty".into());
    }
    if name.contains('.') || name.contains(char::is_whitespace) {
        return Err(format!("invalid name '{name}'"));
    }
    Ok(())
}

fn ensure_unique_name(doc: &Document, name: &str) -> Result<(), String> {
    if doc.nodes.iter().any(|node| node.name == name) {
        return Err(format!("duplicate node name '{name}'"));
    }
    Ok(())
}

fn find_node(doc: &Document, name: &str) -> Option<NodeId> {
    doc.nodes
        .iter()
        .find(|node| node.name == name)
        .map(|node| node.id)
}

fn default_position(doc: &Document) -> [f32; 2] {
    let index = doc.nodes.len();
    [
        40.0 + (index % 3) as f32 * 240.0,
        80.0 + (index / 3) as f32 * 140.0,
    ]
}

fn split_at_suffix(input: &str) -> (String, Option<[f32; 2]>) {
    if let Some((head, tail)) = input.rsplit_once(" at ") {
        if let Some(position) = parse_position(tail) {
            return (head.trim().to_string(), Some(position));
        }
    }
    (input.trim().to_string(), None)
}

fn parse_position(input: &str) -> Option<[f32; 2]> {
    let (x, y) = input.split_once(',')?;
    Some([x.trim().parse().ok()?, y.trim().parse().ok()?])
}

fn parse_port(input: &str, direction: &str) -> Result<PortSpec, String> {
    let (name, ty) = input
        .split_once(':')
        .ok_or_else(|| format!("expected '<name>: <type>' for {direction} port"))?;
    let name = name.trim();
    validate_name(name)?;
    let ty = match ty.trim() {
        "stream" => PortType::Stream,
        "cell" => PortType::Cell,
        "any" => PortType::Any,
        other => return Err(format!("unknown port type '{other}'")),
    };
    Ok(PortSpec {
        name: name.to_string(),
        ty,
    })
}

fn resolve_port(doc: &Document, spec: &str, target: bool) -> Result<(NodeId, usize), String> {
    let (name, port) = match spec.split_once('.') {
        Some((name, port)) => (name, Some(port)),
        None => (spec, None),
    };
    let id = find_node(doc, name).ok_or_else(|| format!("unknown node '{name}'"))?;
    let node = doc.node(id).ok_or("unknown node")?;
    let types = if target {
        node.kind.input_types()
    } else {
        node.kind.output_types()
    };
    let index = match port {
        None => 0,
        Some("") => {
            return Err(format!("empty port on '{name}'"));
        }
        Some(port) => {
            if let Ok(index) = port.parse::<usize>() {
                index
            } else if target {
                match &node.kind {
                    NodeKind::Merge => match port {
                        "left" => 0,
                        "right" => 1,
                        _ => return Err(format!("unknown merge port '{port}'")),
                    },
                    NodeKind::Snapshot { .. } => match port {
                        "event" => 0,
                        "cell" => 1,
                        _ => return Err(format!("unknown snapshot port '{port}'")),
                    },
                    NodeKind::Lift { .. } => match port {
                        "a" => 0,
                        "b" => 1,
                        _ => return Err(format!("unknown lift port '{port}'")),
                    },
                    NodeKind::Component { inputs, .. } => inputs
                        .iter()
                        .position(|port_spec| port_spec.name == port)
                        .ok_or_else(|| format!("unknown input port '{port}' on '{name}'"))?,
                    _ if port == "in" => 0,
                    _ => return Err(format!("unknown input port '{port}' on '{name}'")),
                }
            } else {
                match &node.kind {
                    NodeKind::Component { outputs, .. } => outputs
                        .iter()
                        .position(|port_spec| port_spec.name == port)
                        .ok_or_else(|| format!("unknown output port '{port}' on '{name}'"))?,
                    _ if port == "out" => 0,
                    _ => return Err(format!("unknown output port '{port}' on '{name}'")),
                }
            }
        }
    };
    if types.is_empty() {
        return Err(if target {
            format!("node '{name}' has no input ports")
        } else {
            format!("node '{name}' has no output")
        });
    }
    if index >= types.len() {
        return Err(format!("node '{name}' has no port {index}"));
    }
    Ok((id, index))
}

fn parse_timed_values(input: &str) -> Result<Vec<TimedValue>, String> {
    if input.trim().is_empty() {
        return Ok(Vec::new());
    }
    input
        .split(',')
        .map(|item| {
            let (time, value) = item.split_once(':').ok_or("expected <time>:<value>")?;
            let time = time
                .trim()
                .parse::<f64>()
                .map_err(|_| format!("invalid time '{}'", time.trim()))?;
            Ok(TimedValue::new(time, parse_value(value.trim())))
        })
        .collect()
}

fn parse_value(input: &str) -> Value {
    let text = input.trim();
    if let Some(inner) = text
        .strip_prefix('"')
        .and_then(|text| text.strip_suffix('"'))
    {
        Value::Text(inner.to_string())
    } else {
        Value::parse(text)
    }
}

fn parse_unary_op(input: &str) -> Result<UnaryOp, String> {
    let mut parts = input.split_whitespace();
    let op = parts.next().ok_or("missing unary operator")?;
    let argument = parts.next();
    if parts.next().is_some() {
        return Err(format!("too many arguments for '{op}'"));
    }
    let number = || {
        argument
            .ok_or_else(|| format!("'{op}' requires a number"))?
            .parse::<f64>()
            .map_err(|_| format!("invalid number for '{op}'"))
    };
    match op {
        "identity" => Ok(UnaryOp::Identity),
        "add" => Ok(UnaryOp::Add(number()?)),
        "mul" | "multiply" => Ok(UnaryOp::Multiply(number()?)),
        "negate" => Ok(UnaryOp::Negate),
        "reverse" => Ok(UnaryOp::Reverse),
        "upper" => Ok(UnaryOp::Upper),
        "lower" => Ok(UnaryOp::Lower),
        "length" | "len" => Ok(UnaryOp::Length),
        "to_text" | "str" => Ok(UnaryOp::ToText),
        "not" => Ok(UnaryOp::Not),
        "const" | "constant" => Ok(UnaryOp::Constant(number()?)),
        other => Err(format!("unknown unary operator '{other}'")),
    }
}

fn parse_filter_op(input: &str) -> Result<FilterOp, String> {
    let (op, argument) = input
        .split_once(char::is_whitespace)
        .map(|(op, argument)| (op, argument.trim()))
        .unwrap_or((input, ""));
    if argument.is_empty() {
        return match op {
            "always" => Ok(FilterOp::Always),
            "non_zero" => Ok(FilterOp::NonZero),
            "positive" => Ok(FilterOp::Positive),
            "non_empty" => Ok(FilterOp::NonEmpty),
            "not" => Ok(FilterOp::Not),
            other => Err(format!("unknown filter operator '{other}'")),
        };
    }
    match op {
        "greater_than" => {
            Ok(FilterOp::GreaterThan(argument.parse().map_err(|_| {
                "greater_than requires a number".to_string()
            })?))
        }
        "equals" => Ok(FilterOp::Equals(argument.to_string())),
        "contains" => Ok(FilterOp::Contains(argument.to_string())),
        _ => Err(format!("unknown filter operator '{op}'")),
    }
}

fn parse_combine_op(input: &str) -> Result<CombineOp, String> {
    match input.trim() {
        "add" => Ok(CombineOp::Add),
        "subtract" | "sub" => Ok(CombineOp::Subtract),
        "multiply" | "mul" => Ok(CombineOp::Multiply),
        "concat" => Ok(CombineOp::Concat),
        "pair" => Ok(CombineOp::Pair),
        "keep_left" => Ok(CombineOp::KeepLeft),
        "keep_right" => Ok(CombineOp::KeepRight),
        "max" => Ok(CombineOp::Max),
        "min" => Ok(CombineOp::Min),
        other => Err(format!("unknown combine operator '{other}'")),
    }
}

fn take_quoted(input: &str) -> Result<(String, &str), String> {
    let input = input.trim_start();
    let Some(rest) = input.strip_prefix('"') else {
        return Err("expected a quoted string".into());
    };
    let mut out = String::new();
    let mut chars = rest.char_indices();
    while let Some((index, character)) = chars.next() {
        match character {
            '"' => return Ok((out, &rest[index + 1..])),
            '\\' => {
                if let Some((_, escaped)) = chars.next() {
                    out.push(escaped);
                }
            }
            _ => out.push(character),
        }
    }
    Err("unterminated quoted string".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render_svgs;
    use crate::sim::simulate;

    fn stream_values(doc: &Document, name: &str) -> Vec<String> {
        let id = doc.nodes.iter().find(|node| node.name == name).unwrap().id;
        simulate(doc)
            .stream(id)
            .iter()
            .map(|event| event.value.to_string())
            .collect()
    }

    fn cell_values(doc: &Document, name: &str) -> Vec<String> {
        let id = doc.nodes.iter().find(|node| node.name == name).unwrap().id;
        simulate(doc)
            .cell(id)
            .iter()
            .map(|step| step.value.to_string())
            .collect()
    }

    #[test]
    fn parses_the_calibration_example() {
        let parsed = parse(include_str!("../examples/calibration.frp")).unwrap();
        assert!(parsed.auto_layout);
        assert_eq!(
            cell_values(&parsed.doc, "accum"),
            vec!["0", "5", "15", "0", "-3", "5"]
        );
        assert_eq!(
            cell_values(&parsed.doc, "scaled"),
            vec!["0", "5", "15", "0", "-6", "10"]
        );
    }

    #[test]
    fn parses_the_component_example() {
        let parsed = parse(include_str!("../examples/component.frp")).unwrap();
        assert_eq!(parsed.doc.components.len(), 1);
        assert!(parsed.doc.nodes.iter().any(|node| node.kind.is_component()));
        assert_eq!(
            stream_values(&parsed.doc, "observed"),
            vec!["5", "15", "0", "-3", "5"]
        );
        let definition = crate::find_component(&parsed.doc, "accumulate").unwrap();
        assert_eq!(definition.outputs.len(), 2);
        let expanded = crate::render_component_network(
            &parsed.doc,
            "accumulate",
            &crate::RenderOptions::default(),
        )
        .unwrap();
        assert!(expanded.contains(">snapshot</text>"));
    }

    #[test]
    fn parses_the_annotation_example() {
        let parsed = parse(include_str!("../examples/annotation.frp")).unwrap();
        assert_eq!(parsed.doc.annotations.len(), 1);
        let annotation = &parsed.doc.annotations[0];
        assert!(matches!(
            annotation.target,
            Some(AnnotationTarget::Node { .. })
        ));
        let bundle = render_svgs(&parsed.doc);
        assert!(bundle.network.contains("each event is incremented by one"));
    }

    #[test]
    fn parses_transaction_notes() {
        let parsed = parse(
            "stream a events 1:1\nmap m = identity\nsink s\na -> m\nm -> s\nnote \"T1\" on tx 1 at 200,70\n",
        )
        .unwrap();
        assert_eq!(parsed.doc.annotations.len(), 1);
        assert!(matches!(
            parsed.doc.annotations[0].target,
            Some(AnnotationTarget::Transaction { index: 0 })
        ));
    }

    #[test]
    fn parses_structural_display_nodes() {
        let parsed = parse(include_str!("../examples/structural.frp")).unwrap();
        assert_eq!(parsed.doc.mode, crate::model::DocumentMode::Structural);
        let update = parsed
            .doc
            .nodes
            .iter()
            .find(|node| node.name == "update")
            .unwrap();
        assert!(update.display_only);
        assert_eq!(update.label, "SetReferenceMapPoints");
        assert!(crate::sim::validate_document(&parsed.doc).errors.is_empty());
        let bundle = render_svgs(&parsed.doc);
        assert!(bundle.network.contains("SetReference"));
        assert!(!bundle.timing.contains(">42</text>"));
    }

    #[test]
    fn display_only_nodes_are_rejected_in_executable_mode() {
        let mut parsed = parse(include_str!("../examples/structural.frp")).unwrap();
        parsed.doc.mode = crate::model::DocumentMode::Executable;
        let report = crate::sim::validate_document(&parsed.doc);
        assert!(report
            .errors
            .iter()
            .any(|error| error.contains("Display-only")));
    }

    #[test]
    fn parses_edge_label_width_and_newlines() {
        let parsed =
            parse("stream a\nmap m = identity\nsink s\na -> m : first\\nsecond @ 80\nm -> s\n")
                .unwrap();
        let edge = parsed.doc.edges.first().unwrap();
        assert_eq!(edge.label, "first\nsecond");
        assert_eq!(edge.label_width, Some(80.0));
    }

    #[test]
    fn reports_unknown_nodes_and_ports() {
        let error = parse("stream a\na -> missing\n").unwrap_err();
        assert!(error.contains("unknown node 'missing'"));

        let error = parse("stream a\nmerge merged\nmerged -> a\n").unwrap_err();
        assert!(error.contains("no input ports"));

        let error = parse("sink s\nmerge merged\ns -> merged\n").unwrap_err();
        assert!(error.contains("no output"));
    }

    #[test]
    fn reports_unclosed_components() {
        let error = parse("component foo {\n  in a: stream\n").unwrap_err();
        assert!(error.contains("unclosed component"));
    }
}
