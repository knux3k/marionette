//! `marionette stability`: writes the reports of the `stability_run` operation to files.

use marionette_core::{Diagnostic, Errors};
use serde_json::{Value, json};
use std::path::Path;

fn write_file(path: &Path, text: &str) -> Result<(), Errors> {
    std::fs::write(path, text).map_err(|e| {
        Errors::from(Diagnostic::error(
            marionette_core::error::codes::IO_WRITE,
            format!("cannot write {}: {e}", path.display()),
        ))
    })
}

/// Writes `report.json`, and per failed mode a crash report and its input as a replayable
/// events file. Returns the result with `command`, `report_dir` and the written files.
pub fn write(out: &str, mut v: Value) -> Result<Value, Errors> {
    let dir = Path::new(out);
    std::fs::create_dir_all(dir).map_err(|e| {
        Errors::from(Diagnostic::error(
            marionette_core::error::codes::IO_WRITE,
            format!("cannot create {out}: {e}"),
        ))
    })?;
    let mut files = Vec::new();
    for r in v["results"].as_array_mut().into_iter().flatten() {
        if r["passed"] == true {
            continue;
        }
        let stem = format!(
            "{}-seed{}",
            r["mode"].as_str().unwrap_or("mode"),
            r["seed"].as_u64().unwrap_or(0)
        );
        if let Some(events) = r.get("input_events").cloned().filter(|e| !e.is_null()) {
            let name = format!("{stem}.inputs.json");
            let file = json!({ "format": "marionette.input-events", "version": 1, "events": events });
            write_file(
                &dir.join(&name),
                &(serde_json::to_string_pretty(&file).expect("json") + "\n"),
            )?;
            r["input_file"] = json!(format!("{out}/{name}"));
            files.push(format!("{out}/{name}"));
        }
        let name = format!("{stem}.crash.json");
        write_file(
            &dir.join(&name),
            &(serde_json::to_string_pretty(r).expect("json") + "\n"),
        )?;
        files.push(format!("{out}/{name}"));
    }
    // The summary report keeps everything except the (possibly long) input lists.
    let mut summary = v.clone();
    for r in summary["results"].as_array_mut().into_iter().flatten() {
        if let Some(o) = r.as_object_mut() {
            o.remove("input_events");
        }
    }
    write_file(
        &dir.join("report.json"),
        &(serde_json::to_string_pretty(&summary).expect("json") + "\n"),
    )?;
    files.insert(0, format!("{out}/report.json"));
    let mut result = json!({ "ok": true, "command": "stability", "report_dir": out, "files": files });
    if let (Value::Object(o), Value::Object(m)) = (&mut result, summary) {
        o.extend(m);
    }
    Ok(result)
}

/// One line per mode run, then the files written.
pub fn human(v: &Value) -> String {
    let mut lines = Vec::new();
    for r in v["results"].as_array().into_iter().flatten() {
        let status = if r["skipped"].is_string() {
            "SKIP"
        } else if r["passed"] == true {
            "ok"
        } else {
            "FAIL"
        };
        let mut line = format!(
            "{status:4} {} seed {} ({} ticks): tick p99 {} ms",
            r["mode"].as_str().unwrap_or(""),
            r["seed"],
            r["ticks"],
            r["tick"]["p99_ms"]
        );
        if let Some(f) = r.get("frame").filter(|f| f.is_object()) {
            line.push_str(&format!(", frame p99 {} ms", f["p99_ms"]));
        }
        if let Some(m) = r.get("memory").filter(|m| m.is_object()) {
            line.push_str(&format!(", memory +{} MiB", m["growth_mib"]));
        }
        if r["frame"].is_object() && r["frame_budget_checked"] == false && v["timing_enforced"] == true {
            line.push_str(" (frame budget not checked: software GPU)");
        }
        if let Some(s) = r["skipped"].as_str() {
            line.push_str(&format!(" [{s}]"));
        }
        lines.push(line);
        for f in r["failures"].as_array().into_iter().flatten() {
            lines.push(format!(
                "     {} at tick {}: {}",
                f["kind"].as_str().unwrap_or(""),
                f["tick"],
                f["message"].as_str().unwrap_or("")
            ));
        }
        if r["passed"] != true {
            lines.push(format!(
                "     reproduce: {}",
                r["reproduce"].as_str().unwrap_or("")
            ));
        }
    }
    lines.push(format!(
        "{} ({} build, timing {}); reports in {}",
        if v["passed"] == true { "PASSED" } else { "FAILED" },
        v["build"].as_str().unwrap_or(""),
        if v["timing_enforced"] == true {
            "checked"
        } else {
            "not checked"
        },
        v["report_dir"].as_str().unwrap_or("")
    ));
    lines.join("\n")
}
