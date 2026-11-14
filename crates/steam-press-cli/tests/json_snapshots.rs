//! Contract snapshots for the stable `--json` output of offline commands.

use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("steam-press-json-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run_json(config: &Path, args: &[&str]) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_steam-press"))
        .arg("--config-dir")
        .arg(config)
        .arg("--work-dir")
        .arg(config.join("work"))
        .arg("--output")
        .arg(config.join("out"))
        .args(args)
        .arg("--json")
        .output()
        .expect("run steam-press");
    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "stdout is not pure JSON: {error}: {}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

fn assert_snapshot(actual: Value, expected: Value) {
    assert_eq!(actual, expected);
}

#[test]
fn queue_and_workshop_snapshots() {
    let config = temp_dir("queues");
    let queue = config.join("queue.sspq");
    assert_snapshot(
        run_json(&config, &["queue", "list"]),
        json!({
            "schemaVersion": 1,
            "ok": true,
            "file": queue.display().to_string(),
            "jobs": [],
            "warnings": []
        }),
    );

    let workshop = config.join("queue.sswq");
    assert_snapshot(
        run_json(&config, &["workshop", "list"]),
        json!({
            "schemaVersion": 1,
            "ok": true,
            "file": workshop.display().to_string(),
            "jobs": [],
            "warnings": []
        }),
    );
}

#[test]
fn depots_login_and_template_snapshots() {
    let config = temp_dir("misc");
    assert_snapshot(
        run_json(&config, &["depots", "list"]),
        json!({
            "schemaVersion": 1,
            "ok": true,
            "count": 0,
            "depots": []
        }),
    );
    assert_snapshot(
        run_json(&config, &["login", "--username", "anonymous"]),
        json!({
            "schemaVersion": 1,
            "ok": true,
            "anonymous": true
        }),
    );

    let job = config.join("job.json");
    let template = config.join("template.txt");
    std::fs::write(
        &job,
        r#"{"game_name":"Test","os":"Linux64","branch":"public","build_id":"1","app_id":"42"}"#,
    )
    .unwrap();
    std::fs::write(&template, "{{game_name}}/{{build_id}}").unwrap();
    assert_snapshot(
        run_json(
            &config,
            &[
                "template",
                "render",
                job.to_str().unwrap(),
                template.to_str().unwrap(),
            ],
        ),
        json!({
            "schemaVersion": 1,
            "ok": true,
            "rendered": "Test/1"
        }),
    );
}

#[test]
fn diff_snapshot() {
    let config = temp_dir("diff");
    let old = config.join("old");
    let new = config.join("new");
    std::fs::create_dir_all(&old).unwrap();
    std::fs::create_dir_all(&new).unwrap();
    std::fs::write(old.join("a"), b"a").unwrap();
    std::fs::write(new.join("b"), b"b").unwrap();

    assert_snapshot(
        run_json(
            &config,
            &["diff", old.to_str().unwrap(), new.to_str().unwrap()],
        ),
        json!({
            "schemaVersion": 1,
            "ok": true,
            "stem": "old-to-new",
            "report": {
                "added": [{
                    "kind": "added",
                    "path": "b",
                    "oldSize": null,
                    "newSize": 1
                }],
                "modified": [],
                "removed": [{
                    "kind": "removed",
                    "path": "a",
                    "oldSize": 1,
                    "newSize": null
                }],
                "unchanged": 0
            },
            "reportPath": null,
            "output": null
        }),
    );
}

#[test]
fn error_output_is_json_and_versioned() {
    let config = temp_dir("error");
    let output = Command::new(env!("CARGO_BIN_EXE_steam-press"))
        .arg("--config-dir")
        .arg(&config)
        .args(["depots", "lookup", "0", "--json"])
        .output()
        .expect("run steam-press");
    assert_eq!(output.status.code(), Some(1));
    let document: Value = serde_json::from_slice(&output.stdout).expect("JSON error document");
    assert_eq!(document["schemaVersion"], 1);
    assert_eq!(document["ok"], false);
    assert!(document["error"].as_str().is_some());
}
