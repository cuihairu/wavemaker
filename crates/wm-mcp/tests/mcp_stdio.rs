//! `wm-mcp` stdio end-to-end: the official rmcp client really spawns this
//! crate's binary and walks initialize → tools/list → tools/call over
//! stdio (WM-MCP-001 acceptance; same discipline as the sandtable-mcp
//! `mcp_stdio.rs` precedent — no test stubs, the same chain an MCP
//! proxy/IDE uses). The official client parses every JSON-RPC frame, so a
//! green run asserts frame legality on the whole exchange.

use rmcp::{
    model::{CallToolRequestParams, ContentBlock},
    transport::TokioChildProcess,
    ServiceExt,
};
use serde_json::{json, Value};
use tokio::process::Command;

const RAMP_YAML: &str = "arrival:\n  ramp:\n    from_per_second: 1\n    to_per_second: 20\n    duration_secs: 600\nlifetime:\n  distribution: fixed\n  mean_secs: 900\n";

#[tokio::test]
async fn mcp_stdio_end_to_end_initialize_list_call() -> anyhow::Result<()> {
    let bin = env!("CARGO_BIN_EXE_wm-mcp");
    // TokioChildProcess::new returns a Result (spawn failures are
    // reportable); take the transport first.
    let client = ().serve(TokioChildProcess::new(Command::new(bin))?).await?;

    // 1) tools/list: exactly the seven tools (WM-MCP-001's five plus the
    //    WM-TP-2 plan pair), no phantom tools for sources that do not
    //    exist (live metrics/report have no engine data source yet —
    //    WM-0412/0413/0416 — so there are no such tools).
    let tools = client.list_all_tools().await?;
    let mut names: Vec<String> = tools.iter().map(|t| t.name.to_string()).collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            "get_test_report",
            "list_runs",
            "load_timeline",
            "query_status",
            "run_scenario",
            "run_test_plan",
            "validate_spectrum"
        ],
        "tools = {names:?}"
    );

    // 2) tools/call: load_timeline is pure engine math — no controller, no
    //    filesystem. The result must be legal JSON with the same shape the
    //    desktop timeline command returns (WM-0604).
    let params = CallToolRequestParams::new("load_timeline".to_string())
        .with_arguments(json!({ "content": RAMP_YAML }).as_object().unwrap().clone());
    let result = client.call_tool(params).await?;
    assert!(result.is_error != Some(true), "call must succeed");
    let text = match result.content.first() {
        Some(ContentBlock::Text(t)) => t.text.clone(),
        other => panic!("unexpected content block: {other:?}"),
    };
    let tl: Value = serde_json::from_str(&text)?;
    assert_eq!(tl["points"].as_array().unwrap().len(), 121);
    let horizon = tl["horizon_secs"].as_f64().unwrap();
    assert!((horizon - 1200.0).abs() < 1e-9, "horizon = {horizon}");
    let first = tl["points"][0]["rate_per_second"].as_f64().unwrap();
    assert!((first - 1.0).abs() < 1e-9, "first rate = {first}");
    let last = tl["points"][120]["rate_per_second"].as_f64().unwrap();
    assert!((last - 20.0).abs() < 1e-9, "last rate = {last}");

    // 3) tools/call over the filesystem: run_test_plan on a stub plan
    //    (time_scale 0 — no real waiting), then get_test_report reads the
    //    same run directory back through the discovery path.
    let tmp = tempfile::tempdir()?;
    let spec = tmp.path().join("spec.yaml");
    std::fs::write(&spec, RAMP_YAML)?;
    let plan = tmp.path().join("plan.toml");
    std::fs::write(
        &plan,
        format!(
            "[meta]\nname = \"stdio-demo\"\n\n[[steps]]\nid = \"load\"\naction = \"inject_load\"\n[steps.params]\nspectrum = \"{}\"\nconcurrency = 1\nduration_secs = 30\n\n[[steps]]\nid = \"snap\"\naction = \"capture\"\n\n[[steps]]\nid = \"ok\"\naction = \"assert\"\n[steps.params]\nmetric = \"error_rate\"\nmax = 0.0\nsource = \"snap\"\n",
            spec.display()
        ),
    )?;
    let run_dir = tmp.path().join("stdio-run");
    let params = CallToolRequestParams::new("run_test_plan".to_string()).with_arguments(
        json!({
            "path": plan.display().to_string(),
            "backend": "stub",
            "time_scale": 0.0,
            "out": run_dir.display().to_string(),
        })
        .as_object()
        .unwrap()
        .clone(),
    );
    let result = client.call_tool(params).await?;
    assert!(result.is_error != Some(true), "stub plan must run green");
    let text = match result.content.first() {
        Some(ContentBlock::Text(t)) => t.text.clone(),
        other => panic!("unexpected content block: {other:?}"),
    };
    let outcome: Value = serde_json::from_str(&text)?;
    assert_eq!(outcome["verdict"], "passed", "{}", outcome);
    assert_eq!(outcome["backend"], "stub");
    assert_eq!(outcome["plan_name"], "stdio-demo");
    assert_eq!(outcome["asserts"][0]["judgement"]["verdict"], "pass");
    assert!(run_dir.join("report.md").exists());

    let params = CallToolRequestParams::new("get_test_report".to_string()).with_arguments(
        json!({ "run_dir": run_dir.display().to_string() })
            .as_object()
            .unwrap()
            .clone(),
    );
    let result = client.call_tool(params).await?;
    assert!(result.is_error != Some(true), "report fetch must succeed");
    let text = match result.content.first() {
        Some(ContentBlock::Text(t)) => t.text.clone(),
        other => panic!("unexpected content block: {other:?}"),
    };
    let report: Value = serde_json::from_str(&text)?;
    assert_eq!(report["run"]["verdict"], "passed");
    assert!(
        report["report_markdown"]
            .as_str()
            .unwrap()
            .contains("# Test plan report: stdio-demo"),
        "{}",
        report
    );

    client.cancel().await?;
    Ok(())
}
