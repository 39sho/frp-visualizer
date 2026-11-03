use std::path::PathBuf;
use std::process::Command;

fn bin() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_frp-visualizer"));
    command.current_dir(env!("CARGO_MANIFEST_DIR"));
    command
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "frp_visualizer_cli_{}_{}",
        std::process::id(),
        name
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::create_dir_all(&dir);
    dir
}

#[test]
fn version_and_help_are_available() {
    let output = bin().arg("--version").output().unwrap();
    assert!(output.status.success());
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains(env!("CARGO_PKG_VERSION")), "{text}");

    let output = bin().arg("help").output().unwrap();
    assert!(output.status.success());
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("render <file.frp|file.json>"), "{text}");
    assert!(text.contains("timing-scope"), "{text}");
}

#[test]
fn init_creates_a_starter_dsl_file() {
    let dir = temp_dir("init");
    let path = dir.join("starter.frp");
    let output = bin().arg("init").arg(&path).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("diagram My FRP network"));
    assert!(text.contains("stream sInput"));

    let output = bin().arg("init").arg(&path).output().unwrap();
    assert!(!output.status.success());

    let output = bin()
        .arg("init")
        .arg(&path)
        .arg("--force")
        .output()
        .unwrap();
    assert!(output.status.success());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn check_and_render_a_valid_example() {
    let output = bin()
        .args(["check", "examples/component.frp", "--strict"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let dir = temp_dir("render");
    let output = bin()
        .arg("render")
        .arg("examples/component.frp")
        .arg(&dir)
        .args([
            "--timing-scope",
            "all",
            "--timing-values",
            "none",
            "--focus-transaction",
            "1",
            "--expand-all",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for name in [
        "network.svg",
        "timing.svg",
        "diagram.svg",
        "diagram.json",
        "trace.json",
        "timing_accumulate.svg",
        "expanded_accumulate.svg",
    ] {
        assert!(dir.join(name).exists(), "missing {name}");
    }
    let trace: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("trace.json")).unwrap()).unwrap();
    assert_eq!(trace["values_reliable"], true);
    assert!(!trace["lanes"].as_array().unwrap().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn modular_component_project_compiles_and_renders() {
    let dir = temp_dir("modular");
    let json_path = dir.join("compiled.json");
    let output = bin()
        .args(["check", "examples/modular/main.frp", "--strict"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let output = bin()
        .arg("compile")
        .arg("examples/modular/main.frp")
        .arg(&json_path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let compiled: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&json_path).unwrap()).unwrap();
    let component_names: Vec<&str> = compiled["components"]
        .as_array()
        .unwrap()
        .iter()
        .map(|component| component["name"].as_str().unwrap())
        .collect();
    assert!(component_names.contains(&"accumulator"));
    assert!(component_names.contains(&"pipeline"));

    let output = bin()
        .arg("render")
        .arg("examples/modular/main.frp")
        .arg(&dir)
        .args(["--timing-scope", "all", "--expand-all"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for name in [
        "network.svg",
        "timing.svg",
        "diagram.svg",
        "diagram.json",
        "trace.json",
        "timing_pipeline.svg",
        "timing_pipeline_acc.svg",
        "expanded_accumulator.svg",
        "expanded_pipeline.svg",
    ] {
        assert!(dir.join(name).exists(), "missing {name}");
    }
    let trace: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("trace.json")).unwrap()).unwrap();
    assert_eq!(trace["values_reliable"], true);

    for name in ["expanded_pipeline.json", "expanded_accumulator.json"] {
        let output = bin()
            .arg("check")
            .arg(dir.join(name))
            .arg("--strict")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let from_json = dir.join("from_json");
    let output = bin()
        .arg("render")
        .arg(&json_path)
        .arg(&from_json)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(from_json.join("network.svg").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn invalid_document_is_not_rendered() {
    let dir = temp_dir("invalid");
    let out = dir.join("out");
    let output = bin()
        .arg("check")
        .arg("tests/fixtures/missing-input.frp")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("input connected"));

    let output = bin()
        .arg("render")
        .arg("tests/fixtures/missing-input.frp")
        .arg(&out)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("refusing to render"));
    assert!(!out.join("network.svg").exists());
    let _ = std::fs::remove_dir_all(&dir);
}
