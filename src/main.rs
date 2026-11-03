use frp_visualizer::execution::{build_execution_model, ScopeId};
use frp_visualizer::layout::{LayoutOptions, RouteStyle};
use frp_visualizer::model::Document;
use frp_visualizer::render_timing::{TimingNetworkMode, TimingOptions};
use frp_visualizer::{
    dsl, examples, find_component, layout, load_document, render_component_network,
    render_svgs_with_options, render_timing_scope_svg, save_document, RenderOptions,
};
use std::path::PathBuf;

#[derive(Clone, Debug, Default)]
enum TimingScopeArg {
    #[default]
    Outer,
    All,
    Component(String),
}

struct RenderArgs {
    out_dir: String,
    auto_layout: bool,
    focus: Option<usize>,
    route_style: RouteStyle,
    expand: Option<String>,
    expand_all: bool,
    timing_scope: TimingScopeArg,
    show_values: bool,
    timing_network: TimingNetworkMode,
    network_only: bool,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(command) = args.first().map(String::as_str) else {
        usage();
        std::process::exit(2);
    };
    match command {
        "help" | "-h" | "--help" => usage(),
        "list" => {
            for (label, key) in examples::EXAMPLES {
                println!("{key:<14} {label}");
            }
        }
        "example" => {
            let Some(name) = args.get(1) else {
                eprintln!("usage: frp-visualizer example <name> [out.json]");
                std::process::exit(2);
            };
            let Some(doc) = examples::load_example(name) else {
                eprintln!("unknown example: {name}");
                std::process::exit(2);
            };
            let json = serde_json::to_string_pretty(&doc).expect("serialize document");
            if let Some(out) = args.get(2) {
                if let Err(err) = std::fs::write(out, json) {
                    eprintln!("failed to write {out}: {err}");
                    std::process::exit(1);
                }
                println!("wrote {out}");
            } else {
                println!("{json}");
            }
        }
        "render" => {
            let Some(input) = args.get(1) else {
                eprintln!(
                    "usage: frp-visualizer render <diagram.json|diagram.frp> [out_dir] [--auto-layout] [--focus-transaction N]"
                );
                std::process::exit(2);
            };
            let path = PathBuf::from(input);
            let doc = match load_input(&path) {
                Ok(doc) => doc,
                Err(err) => {
                    eprintln!("failed to load {input}: {err}");
                    std::process::exit(1);
                }
            };
            render_to_dir(doc, render_args(&args, 2));
        }
        "compile" => {
            let Some(input) = args.get(1) else {
                eprintln!("usage: frp-visualizer compile <diagram.frp> [out.json] [--auto-layout]");
                std::process::exit(2);
            };
            let path = PathBuf::from(input);
            let parsed = match dsl::parse_file(&path) {
                Ok(parsed) => parsed,
                Err(err) => {
                    eprintln!("failed to parse {input}: {err}");
                    std::process::exit(1);
                }
            };
            let mut doc = parsed.doc;
            if args.iter().skip(2).any(|arg| arg == "--auto-layout") {
                layout::auto_layout_tree(&mut doc);
            }
            let out = args
                .iter()
                .skip(2)
                .find(|arg| !arg.starts_with("--"))
                .map(PathBuf::from)
                .unwrap_or_else(|| path.with_extension("json"));
            match save_document(&doc, &out) {
                Ok(()) => println!("wrote {}", out.display()),
                Err(err) => {
                    eprintln!("failed to write {}: {err}", out.display());
                    std::process::exit(1);
                }
            }
        }
        "check" => {
            let Some(input) = args.get(1) else {
                eprintln!("usage: frp-visualizer check <diagram.frp|diagram.json>");
                std::process::exit(2);
            };
            let path = PathBuf::from(input);
            let doc = match load_input(&path) {
                Ok(doc) => doc,
                Err(err) => {
                    eprintln!("failed to load {input}: {err}");
                    std::process::exit(1);
                }
            };
            let strict = args.iter().skip(2).any(|arg| arg == "--strict");
            let report = frp_visualizer::sim::validate_document(&doc);
            for error in &report.errors {
                eprintln!("error: {error}");
            }
            for warning in &report.warnings {
                eprintln!("warning: {warning}");
            }
            println!(
                "{}: {} nodes, {} edges, {} components, {} annotations, {} errors, {} warnings",
                doc.title,
                doc.nodes.len(),
                doc.edges.len(),
                doc.components.len(),
                doc.annotations.len(),
                report.errors.len(),
                report.warnings.len()
            );
            println!("timing_reliable: {}", report.timing_reliable);
            if !report.errors.is_empty() || (strict && !report.warnings.is_empty()) {
                std::process::exit(2);
            }
        }
        "render-example" => {
            let Some(name) = args.get(1) else {
                eprintln!(
                    "usage: frp-visualizer render-example <name> [out_dir] [--auto-layout] [--focus-transaction N]"
                );
                std::process::exit(2);
            };
            let Some(doc) = examples::load_example(name) else {
                eprintln!("unknown example: {name}");
                std::process::exit(2);
            };
            render_to_dir(doc, render_args(&args, 2));
        }
        other => {
            if other.ends_with(".json") || other.ends_with(".frp") {
                let path = PathBuf::from(other);
                match load_input(&path) {
                    Ok(doc) => render_to_dir(doc, render_args(&args, 1)),
                    Err(err) => {
                        eprintln!("failed to load {other}: {err}");
                        std::process::exit(1);
                    }
                }
            } else {
                eprintln!("unknown command: {other}");
                usage();
                std::process::exit(2);
            }
        }
    }
}

fn load_input(path: &std::path::Path) -> Result<Document, String> {
    if path.extension().and_then(|extension| extension.to_str()) == Some("frp") {
        dsl::parse_file(path).map(|parsed| parsed.doc)
    } else {
        load_document(path)
    }
}

fn render_args(args: &[String], start: usize) -> RenderArgs {
    let mut out_dir = None;
    let mut auto_layout = false;
    let mut focus = None;
    let mut route_style = RouteStyle::default();
    let mut expand = None;
    let mut expand_all = false;
    let mut timing_scope = TimingScopeArg::Outer;
    let mut show_values = true;
    let mut timing_network = TimingNetworkMode::Focus;
    let mut network_only = false;
    let mut iter = args.iter().skip(start);
    while let Some(arg) = iter.next() {
        if arg == "--auto-layout" {
            auto_layout = true;
        } else if arg == "--focus-transaction" {
            let Some(value) = iter.next() else {
                eprintln!("--focus-transaction requires a 1-based transaction number");
                std::process::exit(2);
            };
            focus = Some(parse_focus(value));
        } else if let Some(value) = arg.strip_prefix("--focus-transaction=") {
            focus = Some(parse_focus(value));
        } else if arg == "--route" {
            let Some(value) = iter.next() else {
                eprintln!("--route requires hybrid or orthogonal");
                std::process::exit(2);
            };
            route_style = parse_route(value);
        } else if let Some(value) = arg.strip_prefix("--route=") {
            route_style = parse_route(value);
        } else if arg == "--expand" {
            let Some(value) = iter.next() else {
                eprintln!("--expand requires a component name");
                std::process::exit(2);
            };
            expand = Some(value.clone());
        } else if let Some(value) = arg.strip_prefix("--expand=") {
            expand = Some(value.to_string());
        } else if arg == "--expand-all" {
            expand_all = true;
        } else if arg == "--timing-scope" {
            let Some(value) = iter.next() else {
                eprintln!("--timing-scope requires outer, all, or component:<name>");
                std::process::exit(2);
            };
            timing_scope = parse_timing_scope(value);
        } else if let Some(value) = arg.strip_prefix("--timing-scope=") {
            timing_scope = parse_timing_scope(value);
        } else if arg == "--timing-values" {
            let Some(value) = iter.next() else {
                eprintln!("--timing-values requires values or none");
                std::process::exit(2);
            };
            show_values = parse_timing_values(value);
        } else if let Some(value) = arg.strip_prefix("--timing-values=") {
            show_values = parse_timing_values(value);
        } else if arg == "--no-timing-values" {
            show_values = false;
        } else if arg == "--timing-network" {
            let Some(value) = iter.next() else {
                eprintln!("--timing-network requires focus or none");
                std::process::exit(2);
            };
            timing_network = parse_timing_network(value);
        } else if let Some(value) = arg.strip_prefix("--timing-network=") {
            timing_network = parse_timing_network(value);
        } else if arg == "--output" {
            let Some(value) = iter.next() else {
                eprintln!("--output requires all or network");
                std::process::exit(2);
            };
            network_only = parse_output(value);
        } else if let Some(value) = arg.strip_prefix("--output=") {
            network_only = parse_output(value);
        } else if arg == "--network-only" {
            network_only = true;
        } else if out_dir.is_none() {
            out_dir = Some(arg.clone());
        }
    }
    RenderArgs {
        out_dir: out_dir.unwrap_or_else(|| "frp_export".into()),
        auto_layout,
        focus,
        route_style,
        expand,
        expand_all,
        timing_scope,
        show_values,
        timing_network,
        network_only,
    }
}

fn parse_timing_network(value: &str) -> TimingNetworkMode {
    match value {
        "focus" => TimingNetworkMode::Focus,
        "none" => TimingNetworkMode::None,
        other => {
            eprintln!("unknown timing network mode: {other} (use focus or none)");
            std::process::exit(2);
        }
    }
}

fn parse_output(value: &str) -> bool {
    match value {
        "all" => false,
        "network" => true,
        other => {
            eprintln!("unknown output mode: {other} (use all or network)");
            std::process::exit(2);
        }
    }
}

fn parse_timing_scope(value: &str) -> TimingScopeArg {
    match value {
        "outer" => TimingScopeArg::Outer,
        "all" => TimingScopeArg::All,
        other => match other.strip_prefix("component:") {
            Some(name) if !name.is_empty() => TimingScopeArg::Component(name.to_string()),
            _ => {
                eprintln!("unknown timing scope: {other}");
                std::process::exit(2);
            }
        },
    }
}

fn parse_timing_values(value: &str) -> bool {
    match value {
        "values" | "all" => true,
        "none" | "off" => false,
        other => {
            eprintln!("unknown timing values mode: {other} (use values or none)");
            std::process::exit(2);
        }
    }
}

fn collect_component_names(doc: &Document, names: &mut Vec<String>) {
    for definition in &doc.components {
        names.push(definition.name.clone());
        collect_component_names(&definition.doc, names);
    }
}

fn safe_name(name: &str) -> String {
    name.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn parse_route(value: &str) -> RouteStyle {
    match value {
        "orthogonal" | "ortho" => RouteStyle::Orthogonal,
        "hybrid" | "diagonal" => RouteStyle::Hybrid,
        other => {
            eprintln!("unknown route style: {other} (use hybrid or orthogonal)");
            std::process::exit(2);
        }
    }
}

fn parse_focus(value: &str) -> usize {
    let index = value.parse::<usize>().unwrap_or(0);
    if index == 0 {
        eprintln!("--focus-transaction is 1-based; use 1 for T1");
        std::process::exit(2);
    }
    index - 1
}

fn render_to_dir(mut doc: Document, args: RenderArgs) {
    if args.auto_layout {
        layout::auto_layout_tree(&mut doc);
    }
    let options = RenderOptions {
        timing: TimingOptions {
            focus: args.focus,
            show_values: args.show_values,
            timing_network: args.timing_network,
            ..Default::default()
        },
        layout: LayoutOptions {
            route_style: args.route_style,
        },
    };
    let report = frp_visualizer::sim::validate_document(&doc);
    for error in &report.errors {
        eprintln!("error: {error}");
    }
    for warning in &report.warnings {
        eprintln!("warning: {warning}");
    }
    if !report.errors.is_empty() {
        eprintln!("refusing to render an invalid document; fix errors or use `check` for details");
        std::process::exit(2);
    }
    let model = build_execution_model(&doc);
    let dir = PathBuf::from(&args.out_dir);
    if let Err(err) = std::fs::create_dir_all(&dir) {
        eprintln!("failed to create {}: {err}", dir.display());
        std::process::exit(1);
    }
    let bundle = render_svgs_with_options(&doc, &options);
    let files: Vec<(&str, &str)> = if args.network_only {
        vec![("network.svg", bundle.network.as_str())]
    } else {
        vec![
            ("network.svg", bundle.network.as_str()),
            ("timing.svg", bundle.timing.as_str()),
            ("diagram.svg", bundle.combined.as_str()),
        ]
    };
    for (name, contents) in files {
        let path = dir.join(name);
        if let Err(err) = std::fs::write(&path, contents) {
            eprintln!("failed to write {}: {err}", path.display());
            std::process::exit(1);
        }
        println!("wrote {}", path.display());
    }
    let json_path = dir.join("diagram.json");
    if let Err(err) = save_document(&doc, &json_path) {
        eprintln!("failed to write {}: {err}", json_path.display());
    } else {
        println!("wrote {}", json_path.display());
    }
    let trace_path = dir.join("trace.json");
    let trace = frp_visualizer::execution::trace_json(&doc, &model, report.timing_reliable);
    if let Err(err) = std::fs::write(&trace_path, trace) {
        eprintln!("failed to write {}: {err}", trace_path.display());
    } else {
        println!("wrote {}", trace_path.display());
    }
    println!(
        "network: {} chars, timing: {} chars, combined: {} chars",
        bundle.network.len(),
        bundle.timing.len(),
        bundle.combined.len()
    );
    let scopes: Vec<ScopeId> = if args.network_only {
        Vec::new()
    } else {
        match &args.timing_scope {
            TimingScopeArg::Outer => Vec::new(),
            TimingScopeArg::All => model
                .component_scopes
                .iter()
                .map(|scope| ScopeId::Component(scope.id))
                .collect(),
            TimingScopeArg::Component(name) => model
                .component_scopes
                .iter()
                .find(|scope| scope.label == *name || scope.path == *name)
                .map(|scope| vec![ScopeId::Component(scope.id)])
                .unwrap_or_else(|| {
                    eprintln!("component scope not found: {name}");
                    Vec::new()
                }),
        }
    };
    let mut used_names = std::collections::HashSet::new();
    for scope in scopes {
        let Some((svg, _, _)) = render_timing_scope_svg(&doc, &model, &scope, &options) else {
            continue;
        };
        let mut name = safe_name(&model.scope_label(&scope, &doc));
        if !used_names.insert(name.clone()) {
            if let ScopeId::Component(id) = scope {
                name = format!("{name}_{id}");
            }
        }
        let file = dir.join(format!("timing_{name}.svg"));
        if let Err(err) = std::fs::write(&file, svg) {
            eprintln!("failed to write {}: {err}", file.display());
        } else {
            println!("wrote {}", file.display());
        }
    }
    let mut expanded_names = Vec::new();
    if let Some(name) = &args.expand {
        expanded_names.push(name.clone());
    }
    if args.expand_all {
        collect_component_names(&doc, &mut expanded_names);
    }
    expanded_names.sort();
    expanded_names.dedup();
    for name in expanded_names {
        match render_component_network(&doc, &name, &options) {
            Some(svg) => {
                let file = dir.join(format!("expanded_{}.svg", safe_name(&name)));
                if let Err(err) = std::fs::write(&file, svg) {
                    eprintln!("failed to write {}: {err}", file.display());
                } else {
                    println!("wrote {}", file.display());
                }
            }
            None => eprintln!("component not found: {name}"),
        }
        if let Some(definition) = find_component(&doc, &name) {
            let json_path = dir.join(format!("expanded_{}.json", safe_name(&name)));
            match save_document(&definition.doc, &json_path) {
                Ok(()) => println!("wrote {}", json_path.display()),
                Err(err) => eprintln!("failed to write {}: {err}", json_path.display()),
            }
        }
    }
}

fn usage() {
    println!(
        "FRP Visualizer\n\
         \n\
         usage:\n\
           frp-visualizer list                  list built-in examples\n\
           frp-visualizer example <name> [out]  write an example as JSON\n\
           frp-visualizer compile <diagram.frp> [out.json] [--auto-layout]\n\
           frp-visualizer check <diagram.frp|diagram.json> [--strict]\n\
           frp-visualizer render <json|frp> [dir] [--auto-layout] [--focus-transaction N] [--route hybrid|orthogonal] [--expand component | --expand-all] [--timing-scope outer|all|component:<name>] [--timing-values values|none] [--timing-network focus|none] [--output all|network]\n\
           frp-visualizer render-example <name> [dir] [--auto-layout] [--focus-transaction N] [--route hybrid|orthogonal] [--expand component | --expand-all] [--timing-scope outer|all|component:<name>] [--timing-values values|none] [--timing-network focus|none] [--output all|network]\n\
         \n\
         default route: orthogonal (use --route hybrid for diagonal wires)\n"
    );
}
