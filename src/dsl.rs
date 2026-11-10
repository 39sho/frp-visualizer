use crate::layout::auto_layout_tree;
use crate::model::{
    Annotation, AnnotationTarget, CombineOp, ComponentDef, Document, FilterOp, NodeId, NodeKind,
    PortSpec, PortType, TimedValue, UnaryOp, Value,
};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct ParsedDsl {
    pub doc: Document,
    pub auto_layout: bool,
}

#[derive(Clone, Debug)]
struct ImportDirective {
    line: usize,
    raw: String,
    path: PathBuf,
}

pub fn parse_file(path: &Path) -> Result<ParsedDsl, String> {
    Loader::new().parse_root(path)
}

pub fn parse(input: &str) -> Result<ParsedDsl, String> {
    let (imports, body) = strip_leading_imports(input)?;
    if !imports.is_empty() {
        return Err("imports require a source file; use parse_file instead".into());
    }
    parse_text(&body, Vec::new(), false)
}

fn parse_text(
    input: &str,
    definitions: Vec<ComponentDef>,
    imported_only: bool,
) -> Result<ParsedDsl, String> {
    let mut parser = Parser::new(input, definitions, imported_only);
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

fn strip_leading_imports(input: &str) -> Result<(Vec<ImportDirective>, String), String> {
    let mut imports = Vec::new();
    let mut body = String::with_capacity(input.len());
    let mut leading = true;
    for (index, raw) in input.lines().enumerate() {
        let line = raw.trim();
        if leading {
            if line.is_empty() || line.starts_with('#') {
                body.push_str(raw);
                body.push('\n');
                continue;
            }
            if let Some(rest) = line.strip_prefix("import ") {
                let (path, tail) = take_quoted(rest.trim())
                    .map_err(|error| format!("line {}: {error}", index + 1))?;
                if !tail.trim().is_empty() {
                    return Err(format!(
                        "line {}: unexpected text after import path",
                        index + 1
                    ));
                }
                if path.trim().is_empty() {
                    return Err(format!("line {}: import path is empty", index + 1));
                }
                imports.push(ImportDirective {
                    line: index + 1,
                    raw: path.clone(),
                    path: PathBuf::from(path),
                });
                body.push('\n');
                continue;
            }
            leading = false;
        }
        body.push_str(raw);
        body.push('\n');
    }
    Ok((imports, body))
}

fn merge_definitions(target: &mut Vec<ComponentDef>, incoming: Vec<ComponentDef>) {
    for definition in incoming {
        if !target
            .iter()
            .any(|existing| existing.name == definition.name)
        {
            target.push(definition);
        }
    }
}

fn resolve_import(importer: &Path, imported: &Path) -> PathBuf {
    if imported.is_absolute() {
        imported.to_path_buf()
    } else {
        importer
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(imported)
    }
}

fn canonical_file(path: &Path) -> Result<PathBuf, String> {
    std::fs::canonicalize(path)
        .map_err(|error| format!("failed to resolve {}: {error}", path.display()))
}

fn read_source(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))
}

struct Loader {
    cache: HashMap<PathBuf, Vec<ComponentDef>>,
    stack: Vec<PathBuf>,
    origins: HashMap<String, PathBuf>,
}

impl Loader {
    fn new() -> Self {
        Self {
            cache: HashMap::new(),
            stack: Vec::new(),
            origins: HashMap::new(),
        }
    }

    fn parse_root(&mut self, path: &Path) -> Result<ParsedDsl, String> {
        let canonical = canonical_file(path)?;
        self.stack.push(canonical.clone());
        let result = self.parse_root_inner(&canonical);
        self.stack.pop();
        result
    }

    fn parse_root_inner(&mut self, path: &Path) -> Result<ParsedDsl, String> {
        let text = read_source(path)?;
        let (imports, body) = strip_leading_imports(&text)?;
        let mut definitions = Vec::new();
        for import in imports {
            let imported = self.load_import(&import, path)?;
            merge_definitions(&mut definitions, imported);
        }
        parse_text(&body, definitions, false)
    }

    fn load_import(
        &mut self,
        import: &ImportDirective,
        importer: &Path,
    ) -> Result<Vec<ComponentDef>, String> {
        let resolved = resolve_import(importer, &import.path);
        self.load_file(&resolved).map_err(|error| {
            format!(
                "line {}: failed to import \"{}\": {error}",
                import.line, import.raw
            )
        })
    }

    fn load_file(&mut self, path: &Path) -> Result<Vec<ComponentDef>, String> {
        let canonical = canonical_file(path)?;
        if let Some(cached) = self.cache.get(&canonical) {
            return Ok(cached.clone());
        }
        if let Some(index) = self
            .stack
            .iter()
            .position(|candidate| candidate == &canonical)
        {
            let mut cycle: Vec<String> = self.stack[index..]
                .iter()
                .map(|path| path.display().to_string())
                .collect();
            cycle.push(canonical.display().to_string());
            return Err(format!("cyclic import: {}", cycle.join(" -> ")));
        }
        self.stack.push(canonical.clone());
        let result = self.load_file_inner(&canonical);
        self.stack.pop();
        result
    }

    fn load_file_inner(&mut self, path: &Path) -> Result<Vec<ComponentDef>, String> {
        let text = read_source(path)?;
        let (imports, body) =
            strip_leading_imports(&text).map_err(|error| format!("{}: {error}", path.display()))?;
        let mut imported_definitions = Vec::new();
        for import in imports {
            let imported = self.load_import(&import, path)?;
            merge_definitions(&mut imported_definitions, imported);
        }
        let parsed = parse_text(&body, imported_definitions.clone(), true)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        let imported_names: HashSet<&str> = imported_definitions
            .iter()
            .map(|definition| definition.name.as_str())
            .collect();
        let own_definitions: Vec<ComponentDef> = parsed
            .doc
            .components
            .iter()
            .filter(|definition| !imported_names.contains(definition.name.as_str()))
            .cloned()
            .collect();
        for definition in &own_definitions {
            if let Some(existing) = self.origins.get(&definition.name) {
                if existing.as_path() != path {
                    return Err(format!(
                        "duplicate component definition '{}' in {} (already defined in {})",
                        definition.name,
                        path.display(),
                        existing.display()
                    ));
                }
            } else {
                self.origins
                    .insert(definition.name.clone(), path.to_path_buf());
            }
        }
        let mut definitions = imported_definitions;
        merge_definitions(&mut definitions, own_definitions);
        self.cache.insert(path.to_path_buf(), definitions.clone());
        Ok(definitions)
    }
}

struct Parser {
    lines: Vec<(usize, String)>,
    index: usize,
    definitions: Vec<ComponentDef>,
    auto_layout: bool,
    imported_only: bool,
}

impl Parser {
    fn new(input: &str, definitions: Vec<ComponentDef>, imported_only: bool) -> Self {
        let lines = input
            .lines()
            .enumerate()
            .map(|(index, line)| (index + 1, line.trim().to_string()))
            .filter(|(_, line)| !line.is_empty() && !line.starts_with('#'))
            .collect();
        Self {
            lines,
            index: 0,
            definitions,
            auto_layout: false,
            imported_only,
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
            } else if self.imported_only {
                return Err(format!(
                    "imported component file may only contain imports and component definitions (found '{line}')"
                ));
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
        if line.starts_with("import ") {
            return Err("imports must appear at the top of the file".into());
        }
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
            "orElse" | "or_else" => {
                if !tail.is_empty() {
                    return Err("orElse does not take a value".into());
                }
                NodeKind::OrElse
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
                    NodeKind::Merge | NodeKind::OrElse => match port {
                        "left" => 0,
                        "right" => 1,
                        _ => {
                            return Err(format!("unknown {} port '{port}'", node.kind.primitive()))
                        }
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
    use std::path::PathBuf;

    fn temp_project(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "frp_visualizer_dsl_{}_{}",
            std::process::id(),
            name
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_file(path: &Path, text: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, text).unwrap();
    }

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
    fn parses_or_else_and_prefers_left() {
        let parsed = parse(
            "stream a events 1:1, 3:3\nstream b events 1:2, 2:20\norElse merged\nsink out\na -> merged.left\nb -> merged.right\nmerged -> out\n",
        )
        .unwrap();
        let id = parsed
            .doc
            .nodes
            .iter()
            .find(|node| node.name == "merged")
            .unwrap()
            .id;
        assert!(matches!(
            parsed.doc.node(id).unwrap().kind,
            NodeKind::OrElse
        ));
        assert_eq!(stream_values(&parsed.doc, "merged"), vec!["1", "20", "3"]);

        let alias = parse("stream a\nor_else merged\n").unwrap();
        let id = alias
            .doc
            .nodes
            .iter()
            .find(|node| node.name == "merged")
            .unwrap()
            .id;
        assert!(matches!(alias.doc.node(id).unwrap().kind, NodeKind::OrElse));
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
    fn parses_modular_component_files() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/modular/main.frp");
        let parsed = parse_file(&path).unwrap();
        let names: Vec<&str> = parsed
            .doc
            .components
            .iter()
            .map(|definition| definition.name.as_str())
            .collect();
        assert_eq!(
            names.iter().filter(|name| **name == "accumulator").count(),
            1
        );
        assert!(names.contains(&"pipeline"));

        let pipeline = crate::find_component(&parsed.doc, "pipeline").unwrap();
        let instance = pipeline
            .doc
            .nodes
            .iter()
            .find(|node| node.name == "acc")
            .unwrap();
        assert!(matches!(
            &instance.kind,
            NodeKind::Component {
                definition: Some(name),
                ..
            } if name == "accumulator"
        ));

        let bundle = render_svgs(&parsed.doc);
        assert!(bundle.network.contains(">pipeline</text>"));
        let expanded = crate::render_component_network(
            &parsed.doc,
            "pipeline",
            &crate::RenderOptions::default(),
        )
        .unwrap();
        assert!(expanded.contains(">acc</text>"));
        assert!(expanded.contains("snapshot · hold"));
    }

    #[test]
    fn imports_are_relative_and_shared_dependencies_are_loaded_once() {
        let dir = temp_project("diamond");
        write_file(
            &dir.join("shared/base.frp"),
            "component identity {\n  in x: stream\n  out y: stream\n  map m = identity\n  x -> m\n  m -> y\n}\n",
        );
        write_file(
            &dir.join("parts/a.frp"),
            "import \"../shared/base.frp\"\ncomponent a {\n  in x: stream\n  out y: stream\n  component id = identity\n  x -> id.x\n  id.y -> y\n}\n",
        );
        write_file(
            &dir.join("parts/b.frp"),
            "import \"../shared/base.frp\"\ncomponent b {\n  in x: stream\n  out y: stream\n  component id = identity\n  x -> id.x\n  id.y -> y\n}\n",
        );
        write_file(
            &dir.join("main.frp"),
            "import \"parts/a.frp\"\nimport \"parts/b.frp\"\ncomponent top {\n  in x: stream\n  out y: stream\n  component first = a\n  component second = b\n  x -> first.x\n  first.y -> second.x\n  second.y -> y\n}\nstream s events 1:1\ncomponent top = top\ns -> top.x\nmap observed = identity\ntop.y -> observed\nsink out\nobserved -> out\n",
        );

        let parsed = parse_file(&dir.join("main.frp")).unwrap();
        let names: Vec<&str> = parsed
            .doc
            .components
            .iter()
            .map(|definition| definition.name.as_str())
            .collect();
        assert_eq!(names.iter().filter(|name| **name == "identity").count(), 1);
        assert!(names.contains(&"a"));
        assert!(names.contains(&"b"));
        assert!(names.contains(&"top"));
        assert!(render_svgs(&parsed.doc).network.starts_with("<svg"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reports_import_failures_with_context() {
        let dir = temp_project("missing");
        write_file(&dir.join("main.frp"), "import \"missing.frp\"\n");
        let error = parse_file(&dir.join("main.frp")).unwrap_err();
        assert!(
            error.contains("failed to import \"missing.frp\""),
            "{error}"
        );
        assert!(error.contains("missing.frp"), "{error}");

        write_file(&dir.join("literal.frp"), "stream s events 1:1\n");
        write_file(&dir.join("bad_main.frp"), "import \"literal.frp\"\n");
        let error = parse_file(&dir.join("bad_main.frp")).unwrap_err();
        assert!(
            error.contains(
                "imported component file may only contain imports and component definitions"
            ),
            "{error}"
        );

        let error = parse("diagram X\nimport \"literal.frp\"\n").unwrap_err();
        assert!(error.contains("imports must appear at the top"), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_cyclic_and_duplicate_imports() {
        let dir = temp_project("cycles");
        write_file(
            &dir.join("a.frp"),
            "import \"b.frp\"\ncomponent a {\n  in x: stream\n  out y: stream\n  x -> y\n}\n",
        );
        write_file(
            &dir.join("b.frp"),
            "import \"a.frp\"\ncomponent b {\n  in x: stream\n  out y: stream\n  x -> y\n}\n",
        );
        let error = parse_file(&dir.join("a.frp")).unwrap_err();
        assert!(error.contains("cyclic import"), "{error}");

        write_file(
            &dir.join("dup_a.frp"),
            "component duplicated {\n  in x: stream\n  out y: stream\n  x -> y\n}\n",
        );
        write_file(
            &dir.join("dup_b.frp"),
            "component duplicated {\n  in x: stream\n  out y: stream\n  x -> y\n}\n",
        );
        write_file(
            &dir.join("dup_main.frp"),
            "import \"dup_a.frp\"\nimport \"dup_b.frp\"\n",
        );
        let error = parse_file(&dir.join("dup_main.frp")).unwrap_err();
        assert!(
            error.contains("duplicate component definition 'duplicated'"),
            "{error}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reports_unclosed_components() {
        let error = parse("component foo {\n  in a: stream\n").unwrap_err();
        assert!(error.contains("unclosed component"));
    }
}
