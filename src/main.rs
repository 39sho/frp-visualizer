use frp_visualizer::layout::{LayoutOptions, RouteStyle};
use frp_visualizer::model::Document;
use frp_visualizer::render_timing::TimingOptions;
use frp_visualizer::{
    examples, export_svgs_to_dir_with, find_component, layout, load_document,
    render_component_network, render_svgs_with_options, save_document, RenderOptions,
};
use std::path::PathBuf;

struct RenderArgs {
    out_dir: String,
    auto_layout: bool,
    focus: Option<usize>,
    route_style: RouteStyle,
    expand: Option<String>,
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
                    "usage: frp-visualizer render <diagram.json> [out_dir] [--auto-layout] [--focus-transaction N]"
                );
                std::process::exit(2);
            };
            let path = PathBuf::from(input);
            let doc = match load_document(&path) {
                Ok(doc) => doc,
                Err(err) => {
                    eprintln!("failed to load {input}: {err}");
                    std::process::exit(1);
                }
            };
            render_to_dir(doc, render_args(&args, 2));
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
            if other.ends_with(".json") {
                let path = PathBuf::from(other);
                match load_document(&path) {
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

fn render_args(args: &[String], start: usize) -> RenderArgs {
    let mut out_dir = None;
    let mut auto_layout = false;
    let mut focus = None;
    let mut route_style = RouteStyle::default();
    let mut expand = None;
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
        layout::auto_layout(&mut doc);
    }
    for warning in frp_visualizer::sim::simulate(&doc).warnings {
        eprintln!("warning: {warning}");
    }
    let options = RenderOptions {
        timing: TimingOptions {
            focus: args.focus,
            ..Default::default()
        },
        layout: LayoutOptions {
            route_style: args.route_style,
        },
    };
    let dir = PathBuf::from(&args.out_dir);
    match export_svgs_to_dir_with(&doc, &dir, &options) {
        Ok(paths) => {
            for path in paths {
                println!("wrote {}", path.display());
            }
            let json_path = dir.join("diagram.json");
            if let Err(err) = save_document(&doc, &json_path) {
                eprintln!("failed to write {}: {err}", json_path.display());
            } else {
                println!("wrote {}", json_path.display());
            }
            let bundle = render_svgs_with_options(&doc, &options);
            println!(
                "network: {} chars, timing: {} chars, combined: {} chars",
                bundle.network.len(),
                bundle.timing.len(),
                bundle.combined.len()
            );
            if let Some(name) = &args.expand {
                match render_component_network(&doc, name, &options) {
                    Some(svg) => {
                        let file = dir.join(format!("expanded_{}.svg", safe_name(name)));
                        if let Err(err) = std::fs::write(&file, svg) {
                            eprintln!("failed to write {}: {err}", file.display());
                        } else {
                            println!("wrote {}", file.display());
                        }
                    }
                    None => eprintln!("component not found: {name}"),
                }
                if let Some(definition) = find_component(&doc, name) {
                    let json_path = dir.join(format!("expanded_{}.json", safe_name(name)));
                    match save_document(&definition.doc, &json_path) {
                        Ok(()) => println!("wrote {}", json_path.display()),
                        Err(err) => eprintln!("failed to write {}: {err}", json_path.display()),
                    }
                }
            }
        }
        Err(err) => {
            eprintln!("failed to render: {err}");
            std::process::exit(1);
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
           frp-visualizer render <json> [dir] [--auto-layout] [--focus-transaction N] [--route hybrid|orthogonal] [--expand component]\n\
           frp-visualizer render-example <name> [dir] [--auto-layout] [--focus-transaction N] [--route hybrid|orthogonal] [--expand component]\n\
         \n\
         default route: orthogonal (use --route hybrid for diagonal wires)\n"
    );
}
