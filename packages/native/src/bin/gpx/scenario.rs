use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Scenario {
    schema_version: u32,
    name: String,
    app: PathBuf,
    #[serde(default = "default_timeout")]
    timeout_ms: u64,
    steps: Vec<Step>,
}

fn default_timeout() -> u64 {
    5000
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Step {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    timeout_ms: Option<u64>,
    #[serde(flatten)]
    action: Action,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "action", rename_all = "camelCase", deny_unknown_fields)]
enum Action {
    Click {
        #[serde(rename = "testId")]
        test_id: String,
    },
    Press {
        keys: String,
    },
    ExpectText {
        #[serde(rename = "testId")]
        test_id: String,
        text: String,
        #[serde(default)]
        exact: bool,
    },
    ExpectVisible {
        #[serde(rename = "testId")]
        test_id: String,
    },
    ExpectAbsent {
        #[serde(rename = "testId")]
        test_id: String,
    },
    ExpectProp {
        #[serde(rename = "testId")]
        test_id: String,
        prop: String,
        value: Value,
    },
    Window {
        index: usize,
    },
    Screenshot {},
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct StepReport {
    step: Step,
    status: &'static str,
    duration_ms: u128,
    error: Option<String>,
    screenshot: Option<String>,
    tree: Option<String>,
    artifact_errors: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    snapshot: Option<Value>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Report {
    schema_version: u32,
    name: String,
    status: &'static str,
    duration_ms: u128,
    error: Option<String>,
    steps: Vec<StepReport>,
}

struct Application {
    child: Child,
    stderr: Option<thread::JoinHandle<()>>,
}

impl Drop for Application {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(stderr) = self.stderr.take() {
            let _ = stderr.join();
        }
    }
}

struct Client {
    address: SocketAddr,
    token: String,
}

impl Client {
    fn call(&self, command: Value, deadline: Instant) -> Result<Value, String> {
        let timeout = deadline
            .checked_duration_since(Instant::now())
            .filter(|timeout| !timeout.is_zero())
            .ok_or("deadline exceeded")?;
        let mut stream = TcpStream::connect_timeout(&self.address, timeout).map_err(message)?;
        stream.set_read_timeout(Some(timeout)).map_err(message)?;
        stream.set_write_timeout(Some(timeout)).map_err(message)?;
        let body = serde_json::to_vec(&command).map_err(message)?;
        if body.len() > 8192 {
            return Err("command exceeds debug API's 8192-byte limit".into());
        }
        write!(stream, "POST /rpc HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", self.address, self.token, body.len()).map_err(message)?;
        stream.write_all(&body).map_err(message)?;
        let mut bytes = Vec::new();
        stream
            .take(128 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(message)?;
        if bytes.len() > 128 * 1024 * 1024 {
            return Err("debug API response exceeds 128 MiB".into());
        }
        let split = bytes
            .windows(4)
            .position(|part| part == b"\r\n\r\n")
            .ok_or("invalid HTTP response")?;
        let response: Value = serde_json::from_slice(&bytes[split + 4..]).map_err(message)?;
        if let Some(error) = response.get("error") {
            return Err(error.as_str().unwrap_or("debug API error").into());
        }
        response
            .get("result")
            .cloned()
            .ok_or_else(|| "debug API response missing result".into())
    }
}

fn message(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn launch(entry: &Path, output: &Path, timeout: Duration) -> Result<(Application, Client), String> {
    let mut command = Command::new(std::env::current_exe().map_err(message)?);
    command
        .args(["--background", "--dev-server", "0"])
        .arg(entry);
    launch_command(command, output, timeout)
}

fn launch_command(
    mut command: Command,
    output: &Path,
    timeout: Duration,
) -> Result<(Application, Client), String> {
    let mut secret = [0u8; 32];
    getrandom::fill(&mut secret).map_err(message)?;
    let token = secret
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let mut child = command
        .env("GPUIX_DEV_TOKEN", &token)
        .stdout(File::create(output.join("app.stdout.log")).map_err(message)?)
        .stderr(Stdio::piped())
        .spawn()
        .map_err(message)?;
    let stderr = child.stderr.take().ok_or("missing app stderr")?;
    let mut log = match File::create(output.join("app.stderr.log")) {
        Ok(log) => log,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(message(error));
        }
    };
    let (sender, receiver) = mpsc::channel();
    let stderr = thread::spawn(move || {
        let mut sent = false;
        for line in BufReader::new(stderr).lines() {
            let Ok(line) = line else {
                break;
            };
            let _ = writeln!(log, "{line}");
            if !sent {
                if let Some(origin) = line
                    .strip_prefix("gpx: dev API http://")
                    .and_then(|line| line.strip_suffix("/rpc"))
                {
                    if let Ok(address) = origin.parse::<SocketAddr>() {
                        let _ = sender.send(address);
                        sent = true;
                    }
                }
            }
        }
    });
    let app = Application {
        child,
        stderr: Some(stderr),
    };
    let address = receiver
        .recv_timeout(timeout)
        .map_err(|error| format!("application did not start: {error}; see app.stderr.log"))?;
    if !address.ip().is_loopback() {
        return Err("debug API address is not loopback".into());
    }
    Ok((app, Client { address, token }))
}

fn retry<T>(deadline: Instant, mut check: impl FnMut() -> Result<T, String>) -> Result<T, String> {
    loop {
        match check() {
            Ok(value) => return Ok(value),
            Err(error) => {
                thread::sleep(
                    Duration::from_millis(50)
                        .min(deadline.saturating_duration_since(Instant::now())),
                );
                if Instant::now() >= deadline {
                    return Err(format!("timed out: {error}"));
                }
            }
        }
    }
}

fn matches<'tree>(tree: &'tree Value, test_id: &str, found: &mut Vec<&'tree Value>) {
    if tree.get("testId").and_then(Value::as_str) == Some(test_id) {
        found.push(tree);
    }
    if let Some(children) = tree.get("children").and_then(Value::as_array) {
        for child in children {
            matches(child, test_id, found);
        }
    }
}

fn unique<'tree>(tree: &'tree Value, test_id: &str) -> Result<&'tree Value, String> {
    let mut found = Vec::new();
    matches(tree, test_id, &mut found);
    if found.len() != 1 {
        return Err(format!(
            "testId {test_id:?}: expected one node, found {}",
            found.len()
        ));
    }
    Ok(found[0])
}

fn text(tree: &Value) -> String {
    let mut value = tree
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if let Some(children) = tree.get("children").and_then(Value::as_array) {
        for child in children {
            value.push_str(&text(child));
        }
    }
    value
}

fn bounds(
    client: &Client,
    window: u64,
    node: &Value,
    deadline: Instant,
) -> Result<(f64, f64), String> {
    let id = node
        .get("id")
        .and_then(Value::as_u64)
        .ok_or("node missing id")?;
    let geometry = client.call(json!({"method":"geometry", "window":window}), deadline)?;
    let rect = geometry
        .get(id.to_string())
        .ok_or("node has no painted bounds")?;
    let number = |field| {
        rect.get(field)
            .and_then(Value::as_f64)
            .ok_or_else(|| format!("missing bounds {field}"))
    };
    let (width, height) = (number("width")?, number("height")?);
    if width <= 0.0 || height <= 0.0 {
        return Err("node has empty painted bounds".into());
    }
    Ok((number("x")? + width / 2.0, number("y")? + height / 2.0))
}

fn perform(
    client: &Client,
    window: &mut u64,
    action: &Action,
    deadline: Instant,
) -> Result<(), String> {
    if let Action::Window { index } = action {
        *window = retry(deadline, || {
            client
                .call(json!({"method":"windows"}), deadline)?
                .get(index)
                .and_then(Value::as_u64)
                .ok_or_else(|| format!("window index {index} is not open"))
        })?;
        return Ok(());
    }
    if let Action::Press { keys } = action {
        client.call(
            json!({"method":"press", "window":window, "keys":keys}),
            deadline,
        )?;
        return Ok(());
    }
    if let Action::Screenshot {} = action {
        return Ok(());
    }
    let point = retry(deadline, || {
        let tree = client.call(
            json!({"method":"tree", "window":window, "details":true}),
            deadline,
        )?;
        let test_id = match action {
            Action::Click { test_id }
            | Action::ExpectText { test_id, .. }
            | Action::ExpectVisible { test_id }
            | Action::ExpectAbsent { test_id }
            | Action::ExpectProp { test_id, .. } => test_id,
            _ => unreachable!(),
        };
        if matches!(action, Action::ExpectAbsent { .. }) {
            let mut found = Vec::new();
            matches(&tree, test_id, &mut found);
            return if found.is_empty() {
                Ok(None)
            } else {
                Err(format!("testId {test_id:?} is still mounted"))
            };
        }
        let node = unique(&tree, test_id)?;
        match action {
            Action::Click { .. } | Action::ExpectVisible { .. } => {
                if matches!(action, Action::Click { .. })
                    && ["disabled", "aria-disabled"].iter().any(|prop| {
                        let value = node.get("customProps").and_then(|props| props.get(prop));
                        value == Some(&Value::Bool(true))
                            || value.and_then(Value::as_str) == Some("true")
                    })
                {
                    return Err(format!("testId {test_id:?} is disabled"));
                }
                bounds(client, *window, node, deadline).map(Some)
            }
            Action::ExpectText {
                text: expected,
                exact,
                ..
            } => {
                let actual = text(node);
                if if *exact {
                    actual == *expected
                } else {
                    actual.contains(expected)
                } {
                    Ok(None)
                } else {
                    Err(format!(
                        "testId {test_id:?}: expected text {expected:?}, got {actual:?}"
                    ))
                }
            }
            Action::ExpectProp { prop, value, .. } => {
                let actual = node.get("customProps").and_then(|props| props.get(prop));
                if actual == Some(value) {
                    Ok(None)
                } else {
                    Err(format!(
                        "testId {test_id:?}: expected prop {prop:?} = {value}, got {actual:?}"
                    ))
                }
            }
            _ => unreachable!(),
        }
    })?;
    if let Action::Click { .. } = action {
        let (x, y) = point.ok_or("missing click coordinates")?;
        client.call(
            json!({"method":"click", "window":window, "x":x, "y":y}),
            deadline,
        )?;
    }
    Ok(())
}

fn screenshot(client: &Client, window: u64, path: &Path) -> Result<Value, String> {
    let shot = client.call(
        json!({"method":"screenshot", "window":window}),
        Instant::now() + Duration::from_secs(10),
    )?;
    let data = shot
        .get("data")
        .and_then(Value::as_str)
        .ok_or("screenshot missing data")?;
    let png = base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(message)?;
    fs::write(path, png).map_err(message)?;
    Ok(shot)
}

fn capture(client: &Client, window: u64, output: &Path, index: usize, report: &mut StepReport) {
    let name = format!("step-{:03}.png", index + 1);
    match screenshot(client, window, &output.join(&name)) {
        Ok(shot) => {
            report.screenshot = Some(name);
            report.snapshot = Some(
                json!({"width":shot["width"],"height":shot["height"],"scaleFactor":shot["scaleFactor"]}),
            );
        }
        Err(error) => {
            report.artifact_errors.push(error);
            if report.status == "passed" {
                report.status = "failed";
                report.error = Some("screenshot capture failed".into());
            }
        }
    }
    if report.status == "failed" || report.screenshot.is_some() {
        let name = format!("step-{:03}.tree.json", index + 1);
        let result = client
            .call(
                json!({"method":"tree", "window":window, "details":true}),
                Instant::now() + Duration::from_secs(5),
            )
            .and_then(|tree| {
                let test_id = serde_json::to_value(&report.step.action)
                    .ok()
                    .and_then(|action| {
                        action
                            .get("testId")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    });
                if let Some(test_id) = test_id {
                    if let Ok(node) = unique(&tree, &test_id) {
                        if let Some(snapshot) = &mut report.snapshot {
                            snapshot["testId"] = json!(test_id);
                            if let Ok(geometry) = client.call(
                                json!({"method":"geometry","window":window}),
                                Instant::now() + Duration::from_secs(5),
                            ) {
                                snapshot["target"] = geometry
                                    [node["id"].as_u64().unwrap_or_default().to_string()]
                                .clone();
                            }
                            match &report.step.action {
                                Action::ExpectText { text: expected, .. } => {
                                    snapshot["expected"] = json!(expected);
                                    snapshot["received"] = json!(text(node));
                                }
                                Action::ExpectProp { prop, value, .. } => {
                                    snapshot["expected"] = value.clone();
                                    snapshot["received"] = node["customProps"][prop].clone();
                                }
                                _ => {}
                            }
                        }
                    }
                }
                fs::write(
                    output.join(&name),
                    serde_json::to_vec_pretty(&tree).map_err(message)?,
                )
                .map_err(message)
            });
        match result {
            Ok(()) => report.tree = Some(name),
            Err(error) => report.artifact_errors.push(error),
        }
    }
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn write_report(output: &Path, report: &Report) -> Result<(), String> {
    fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(report).map_err(message)?,
    )
    .map_err(message)?;
    let mut html = format!("<!doctype html><meta charset=utf-8><title>{}</title><style>body{{font:15px system-ui;background:#17191e;color:#eceef2;max-width:960px;margin:40px auto;padding:24px}}article{{border-top:1px solid #484c55;padding:16px 0}}pre{{white-space:pre-wrap;overflow-wrap:anywhere}}a{{color:#a9c5ff}}img{{max-width:100%}}.failed{{color:#ff9696}}.passed{{color:#97dab0}}</style><h1>{}</h1><p class=\"{}\">{} · {} ms</p><p><a href=report.json>JSON report</a> · <a href=app.stderr.log>stderr</a> · <a href=app.stdout.log>stdout</a></p>", escape(&report.name), escape(&report.name), report.status, report.status, report.duration_ms);
    if let Some(error) = &report.error {
        html.push_str(&format!("<pre>{}</pre>", escape(error)));
    }
    for (index, step) in report.steps.iter().enumerate() {
        let label = step
            .step
            .name
            .clone()
            .unwrap_or_else(|| format!("{:?}", step.step.action));
        html.push_str(&format!(
            "<article><h2 class=\"{}\">{}. {} — {} ({} ms)</h2>",
            step.status,
            index + 1,
            escape(&label),
            step.status,
            step.duration_ms
        ));
        if let Some(error) = &step.error {
            html.push_str(&format!("<pre>{}</pre>", escape(error)));
        }
        if let Some(path) = &step.tree {
            html.push_str(&format!("<p><a href=\"{path}\">Retained tree</a></p>"));
        }
        if let Some(path) = &step.screenshot {
            html.push_str(&format!("<a href=\"{path}\"><img src=\"{path}\"></a>"));
        }
        for error in &step.artifact_errors {
            html.push_str(&format!("<pre>Artifact: {}</pre>", escape(error)));
        }
        html.push_str("</article>");
    }
    fs::write(output.join("report.html"), html).map_err(message)
}

fn validate(scenario: &Scenario) -> Result<(), String> {
    if scenario.schema_version != 1 {
        return Err("scenario schemaVersion must be 1".into());
    }
    if scenario.name.trim().is_empty()
        || scenario.app.as_os_str().is_empty()
        || scenario.steps.is_empty()
    {
        return Err("scenario requires a name, app entry and at least one step".into());
    }
    for timeout in std::iter::once(scenario.timeout_ms)
        .chain(scenario.steps.iter().filter_map(|step| step.timeout_ms))
    {
        if !(1..=300_000).contains(&timeout) {
            return Err("timeoutMs must be between 1 and 300000".into());
        }
    }
    Ok(())
}

fn run_single(arguments: &[String]) -> Result<(), String> {
    if arguments
        .iter()
        .any(|argument| matches!(argument.as_str(), "--help" | "-h"))
    {
        println!("Usage: gpx test scenario.json [--output DIRECTORY]");
        return Ok(());
    }
    let path = arguments
        .first()
        .filter(|path| !path.starts_with('-'))
        .ok_or("gpx test requires a scenario JSON file")?;
    let output = match &arguments[1..] {
        [] => PathBuf::from("test-results").join(format!(
            "run-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(message)?
                .as_millis()
        )),
        [option, path] if option == "--output" => PathBuf::from(path),
        _ => return Err("Usage: gpx test scenario.json [--output DIRECTORY]".into()),
    };
    let scenario: Scenario =
        serde_json::from_slice(&fs::read(path).map_err(message)?).map_err(message)?;
    validate(&scenario)?;
    fs::create_dir_all(&output).map_err(message)?;
    let entry = Path::new(path)
        .parent()
        .unwrap_or(Path::new("."))
        .join(&scenario.app);
    let started = Instant::now();
    let mut report = Report {
        schema_version: 1,
        name: scenario.name,
        status: "passed",
        duration_ms: 0,
        error: None,
        steps: Vec::new(),
    };
    let result = fs::canonicalize(entry)
        .map_err(message)
        .and_then(|entry| {
            launch(
                &entry,
                &output,
                Duration::from_millis(scenario.timeout_ms.max(10_000)),
            )
        })
        .and_then(|(_app, client)| {
            let deadline = Instant::now() + Duration::from_millis(scenario.timeout_ms.max(10_000));
            let mut window = retry(deadline, || {
                client
                    .call(json!({"method":"windows"}), deadline)?
                    .get(0)
                    .and_then(Value::as_u64)
                    .ok_or_else(|| "no open window".into())
            })?;
            for (index, step) in scenario.steps.iter().cloned().enumerate() {
                let mut step_report = StepReport {
                    step,
                    status: "skipped",
                    duration_ms: 0,
                    error: None,
                    screenshot: None,
                    tree: None,
                    artifact_errors: Vec::new(),
                    snapshot: None,
                };
                if report.status == "passed" {
                    let start = Instant::now();
                    let deadline = start
                        + Duration::from_millis(
                            step_report.step.timeout_ms.unwrap_or(scenario.timeout_ms),
                        );
                    let result = perform(&client, &mut window, &step_report.step.action, deadline);
                    match result {
                        Ok(()) => step_report.status = "passed",
                        Err(error) => {
                            step_report.status = "failed";
                            step_report.error = Some(error);
                            report.status = "failed";
                        }
                    }
                    if step_report.status == "failed"
                        || matches!(step_report.step.action, Action::Screenshot {})
                    {
                        capture(&client, window, &output, index, &mut step_report);
                        if step_report.status == "failed" {
                            report.status = "failed";
                        }
                    }
                    step_report.duration_ms = start.elapsed().as_millis();
                    println!(
                        "{}: step {} {} ({} ms)",
                        report.name,
                        index + 1,
                        step_report.status,
                        step_report.duration_ms
                    );
                }
                report.steps.push(step_report);
            }
            Ok(())
        });
    if let Err(error) = result {
        report.status = "failed";
        report.error = Some(error);
        for step in scenario.steps {
            report.steps.push(StepReport {
                step,
                status: "skipped",
                duration_ms: 0,
                error: None,
                screenshot: None,
                tree: None,
                artifact_errors: Vec::new(),
                snapshot: None,
            });
        }
    }
    report.duration_ms = started.elapsed().as_millis();
    write_report(&output, &report)?;
    println!("Report: {}", output.join("report.html").display());
    if report.status == "failed" {
        Err("UI scenario failed".into())
    } else {
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Suite {
    schema_version: u32,
    name: String,
    app: PathBuf,
    #[serde(default = "default_timeout")]
    timeout_ms: u64,
    tests: Vec<TestCase>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TestCase {
    name: String,
    timeout_ms: Option<u64>,
    steps: Vec<Step>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CaseReport {
    #[serde(flatten)]
    report: Report,
    reset_duration_ms: Option<f64>,
    directory: String,
    suite: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SuiteReport {
    schema_version: u32,
    name: String,
    status: &'static str,
    duration_ms: u128,
    startup_duration_ms: u128,
    tests: Vec<CaseReport>,
}

fn execute_case(
    scenario: &Scenario,
    client: &Client,
    output: &Path,
    setup: Result<u64, String>,
    screenshots: bool,
) -> Report {
    let started = Instant::now();
    let mut report = Report {
        schema_version: 1,
        name: scenario.name.clone(),
        status: "passed",
        duration_ms: 0,
        error: None,
        steps: Vec::new(),
    };
    let mut window = match setup {
        Ok(window) => window,
        Err(error) => {
            report.status = "failed";
            report.error = Some(error);
            0
        }
    };
    for (index, step) in scenario.steps.iter().cloned().enumerate() {
        let mut item = StepReport {
            step,
            status: "skipped",
            duration_ms: 0,
            error: None,
            screenshot: None,
            tree: None,
            artifact_errors: Vec::new(),
            snapshot: None,
        };
        if report.status == "passed" {
            let started = Instant::now();
            let deadline = started
                + Duration::from_millis(item.step.timeout_ms.unwrap_or(scenario.timeout_ms));
            match perform(client, &mut window, &item.step.action, deadline) {
                Ok(()) => item.status = "passed",
                Err(error) => {
                    item.status = "failed";
                    item.error = Some(error);
                }
            }
            if screenshots
                || item.status == "failed"
                || matches!(item.step.action, Action::Screenshot {})
            {
                capture(client, window, output, index, &mut item);
            }
            if item.status == "failed" {
                report.status = "failed";
            }
            item.duration_ms = started.elapsed().as_millis();
        }
        report.steps.push(item);
    }
    report.duration_ms = started.elapsed().as_millis();
    report
}

fn progress(client: &Client, name: &str, cases: Vec<Value>) -> Result<(), String> {
    let mut batch = Vec::new();
    for case in cases {
        let mut next = batch.clone();
        next.push(case.clone());
        if !batch.is_empty()
            && serde_json::to_vec(&json!({"method":"testReport", "name":name, "cases":next}))
                .map_err(message)?
                .len()
                > 8192
        {
            client.call(
                json!({"method":"testReport", "name":name, "cases":batch}),
                Instant::now() + Duration::from_secs(5),
            )?;
            batch = Vec::new();
        }
        batch.push(case);
    }
    client
        .call(
            json!({"method":"testReport", "name":name, "cases":batch}),
            Instant::now() + Duration::from_secs(5),
        )
        .map(|_| ())
}

fn run_suite(
    path: &Path,
    output: PathBuf,
    suite: Suite,
    viewer: Option<&Client>,
    viewer_name: &str,
    offset: usize,
    previous: Option<&SuiteReport>,
    only_failed: bool,
) -> Result<SuiteReport, String> {
    let ui = viewer.is_some();
    if only_failed && previous.is_some_and(|report| report.status == "passed") {
        return Ok(previous.unwrap().clone());
    }
    if suite.tests.is_empty() || suite.tests.len() > 10_000 {
        return Err("suite requires between 1 and 10000 tests".into());
    }
    let scenarios = suite
        .tests
        .into_iter()
        .map(|case| Scenario {
            schema_version: suite.schema_version,
            name: case.name,
            app: suite.app.clone(),
            timeout_ms: case.timeout_ms.unwrap_or(suite.timeout_ms),
            steps: case.steps,
        })
        .collect::<Vec<_>>();
    for scenario in &scenarios {
        validate(scenario)?;
        if scenario.name.len() > 1024 || scenario.name.chars().any(char::is_control) {
            return Err(
                "test names must not exceed 1024 bytes or contain control characters".into(),
            );
        }
    }
    if suite.name.trim().is_empty()
        || suite.name.chars().any(char::is_control)
        || suite.name.len() > 1024
        || !(1..=300_000).contains(&suite.timeout_ms)
    {
        return Err("suite requires a name and valid timeoutMs".into());
    }
    fs::create_dir_all(&output).map_err(message)?;
    let started = Instant::now();
    let entry = Path::new(path)
        .parent()
        .unwrap_or(Path::new("."))
        .join(&suite.app);
    let launched = fs::canonicalize(entry).map_err(message).and_then(|entry| {
        launch(
            &entry,
            &output,
            Duration::from_millis(suite.timeout_ms.max(10_000)),
        )
    });
    let startup_duration_ms = started.elapsed().as_millis();
    let (_app, client, mut fatal) = match launched {
        Ok((app, client)) => (Some(app), Some(client), None),
        Err(error) => (None, None, Some(error)),
    };
    let mut reports = Vec::new();
    let mut executed = 0;
    for (index, scenario) in scenarios.iter().enumerate() {
        if only_failed {
            if let Some(previous) =
                previous.filter(|report| report.tests[index].report.status == "passed")
            {
                reports.push(previous.tests[index].clone());
                continue;
            }
        }
        let directory = format!("case-{:03}", index + 1);
        let case_output = output.join(&directory);
        fs::create_dir_all(&case_output).map_err(message)?;
        let mut reset_duration = None;
        let setup = if let Some(error) = &fatal {
            Err(error.clone())
        } else {
            let client = client.as_ref().unwrap();
            let deadline = Instant::now() + Duration::from_millis(scenario.timeout_ms.max(10_000));
            let setup = if executed == 0 {
                retry(deadline, || {
                    client
                        .call(json!({"method":"windows"}), deadline)?
                        .get(0)
                        .and_then(Value::as_u64)
                        .ok_or("no open window".into())
                })
            } else {
                client
                    .call(json!({"method":"reset"}), deadline)
                    .and_then(|value| {
                        reset_duration = value["durationMs"].as_f64();
                        value["windows"]
                            .get(0)
                            .and_then(Value::as_u64)
                            .ok_or("reset produced no open window".into())
                    })
            };
            if let Err(error) = &setup {
                fatal = Some(error.clone());
            }
            setup
        };
        executed += 1;
        if let Some(viewer) = viewer {
            progress(
                viewer,
                viewer_name,
                vec![
                    json!({"index":offset + index,"suite":path.display().to_string(),"name":scenario.name,"status":"running"}),
                ],
            )?;
        }
        let report = if let Some(client) = &client {
            execute_case(scenario, client, &case_output, setup, ui)
        } else {
            Report {
                schema_version: 1,
                name: scenario.name.clone(),
                status: "failed",
                duration_ms: 0,
                error: fatal.clone(),
                steps: scenario
                    .steps
                    .iter()
                    .cloned()
                    .map(|step| StepReport {
                        step,
                        status: "skipped",
                        duration_ms: 0,
                        error: None,
                        screenshot: None,
                        tree: None,
                        artifact_errors: Vec::new(),
                        snapshot: None,
                    })
                    .collect(),
            }
        };
        write_report(&case_output, &report)?;
        let html_path = case_output.join("report.html");
        let html = fs::read_to_string(&html_path)
            .map_err(message)?
            .replace("href=app.stderr.log", "href=../app.stderr.log")
            .replace("href=app.stdout.log", "href=../app.stdout.log");
        fs::write(html_path, html).map_err(message)?;
        let reset_label = reset_duration
            .map(|duration| format!("reset {duration:.2} ms"))
            .unwrap_or_else(|| "initial mount".into());
        println!(
            "{}: {} ({} ms; {reset_label})",
            report.name, report.status, report.duration_ms
        );
        if let Some(viewer) = viewer {
            let error = report
                .error
                .as_ref()
                .or_else(|| report.steps.iter().find_map(|step| step.error.as_ref()))
                .map(|error| error.chars().take(1000).collect::<String>());
            progress(
                viewer,
                viewer_name,
                vec![
                    json!({"index":offset + index,"suite":path.display().to_string(),"name":report.name,"status":report.status,"durationMs":report.duration_ms,"error":error,"reportPath":fs::canonicalize(case_output.join("report.json")).map_err(message)?}),
                ],
            )?;
        }
        reports.push(CaseReport {
            report,
            reset_duration_ms: reset_duration,
            directory,
            suite: suite.name.clone(),
        });
    }
    let status = if reports.iter().any(|case| case.report.status == "failed") {
        "failed"
    } else {
        "passed"
    };
    let report = SuiteReport {
        schema_version: 1,
        name: suite.name,
        status,
        duration_ms: started.elapsed().as_millis(),
        startup_duration_ms,
        tests: reports,
    };
    write_suite_report(&output, &report)?;
    Ok(report)
}

fn write_suite_report(output: &Path, report: &SuiteReport) -> Result<(), String> {
    let status = report.status;
    fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&report).map_err(message)?,
    )
    .map_err(message)?;
    let mut html = format!("<!doctype html><meta charset=utf-8><title>{}</title><style>body{{font:15px system-ui;background:#17191e;color:#eceef2;max-width:960px;margin:40px auto;padding:24px}}a{{color:#a9c5ff}}p{{overflow-wrap:anywhere}}</style><h1>{}</h1><p>{status} · {} ms · startup {} ms</p><p><a href=report.json>JSON report</a></p>", escape(&report.name), escape(&report.name), report.duration_ms, report.startup_duration_ms);
    for case in &report.tests {
        let reset_label = case
            .reset_duration_ms
            .map(|duration| format!("reset {duration:.2} ms"))
            .unwrap_or_else(|| "initial mount".into());
        html.push_str(&format!(
            "<p><a href=\"{}/report.html\">{}</a> — {} · {} ms ({reset_label})</p>",
            case.directory,
            escape(&format!("{} · {}", case.suite, case.report.name)),
            case.report.status,
            case.report.duration_ms
        ));
    }
    fs::write(output.join("report.html"), html).map_err(message)?;
    println!("Report: {}", output.join("report.html").display());
    Ok(())
}

fn read_suite(path: &Path, value: Value) -> Result<Suite, String> {
    let suite: Suite = if value.get("tests").is_some() {
        serde_json::from_value(value).map_err(message)?
    } else {
        let scenario: Scenario = serde_json::from_value(value).map_err(message)?;
        Suite {
            schema_version: scenario.schema_version,
            name: scenario.name.clone(),
            app: scenario.app,
            timeout_ms: scenario.timeout_ms,
            tests: vec![TestCase {
                name: scenario.name,
                timeout_ms: None,
                steps: scenario.steps,
            }],
        }
    };
    if suite.tests.is_empty()
        || suite.tests.len() > 10_000
        || suite.name.trim().is_empty()
        || suite.name.len() > 1024
        || suite.name.chars().any(char::is_control)
        || !(1..=300_000).contains(&suite.timeout_ms)
    {
        return Err(format!(
            "{}: invalid suite name, timeout or test count",
            path.display()
        ));
    }
    for case in &suite.tests {
        validate(&Scenario {
            schema_version: suite.schema_version,
            name: case.name.clone(),
            app: suite.app.clone(),
            timeout_ms: case.timeout_ms.unwrap_or(suite.timeout_ms),
            steps: case.steps.clone(),
        })
        .map_err(|error| format!("{}: {error}", path.display()))?;
        if case.name.len() > 1024 || case.name.chars().any(char::is_control) {
            return Err(format!("{}: invalid test name", path.display()));
        }
    }
    Ok(suite)
}

fn discover(directory: &Path) -> Result<Vec<(PathBuf, Suite)>, String> {
    fn visit(directory: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
        for entry in fs::read_dir(directory).map_err(message)? {
            let entry = entry.map_err(message)?;
            let kind = entry.file_type().map_err(message)?;
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                if !matches!(
                    entry.file_name().to_str(),
                    Some("node_modules" | "target" | ".git" | "test-results")
                ) {
                    visit(&entry.path(), files)?;
                }
            } else if kind.is_file()
                && entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "json")
            {
                files.push(entry.path());
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    visit(directory, &mut files)?;
    files.sort();
    let mut suites = Vec::new();
    for path in files {
        let value: Value = serde_json::from_slice(&fs::read(&path).map_err(message)?)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        if value.get("app").is_some()
            && (value.get("tests").is_some() || value.get("steps").is_some())
        {
            let suite = read_suite(&path, value)?;
            suites.push((path, suite));
        }
    }
    if suites.is_empty() {
        return Err(format!(
            "{} contains no scenario or suite JSON files",
            directory.display()
        ));
    }
    Ok(suites)
}

pub(super) fn run(arguments: &[String]) -> Result<(), String> {
    if arguments
        .iter()
        .any(|argument| matches!(argument.as_str(), "--help" | "-h"))
    {
        println!("Usage: gpx test FILE-OR-DIRECTORY [--output DIRECTORY] [--ui [--background]]");
        return Ok(());
    }
    let path = Path::new(
        arguments
            .first()
            .filter(|path| !path.starts_with('-'))
            .ok_or("gpx test requires a scenario file or directory")?,
    );
    let mut output = None;
    let mut ui = false;
    let mut background = false;
    let mut options = arguments[1..].iter();
    while let Some(option) = options.next() {
        match option.as_str() {
            "--ui" => ui = true,
            "--background" => background = true,
            "--output" if output.is_none() => {
                output = Some(PathBuf::from(
                    options.next().ok_or("--output requires a directory")?,
                ))
            }
            _ => return Err(format!("unknown test option {option}")),
        }
    }
    if background && !ui {
        return Err("--background requires --ui (test apps already run in the background)".into());
    }
    let directory = path.is_dir();
    let suites = if directory {
        discover(path)?
    } else {
        let value: Value =
            serde_json::from_slice(&fs::read(path).map_err(message)?).map_err(message)?;
        if value.get("tests").is_none() && !ui {
            return run_single(arguments);
        }
        vec![(path.to_path_buf(), read_suite(path, value)?)]
    };
    let total = suites
        .iter()
        .map(|(_, suite)| suite.tests.len())
        .sum::<usize>();
    if total > 10_000 {
        return Err("a test run supports at most 10000 cases".into());
    }
    let output = output.unwrap_or_else(|| {
        PathBuf::from("test-results").join(format!(
            "suite-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        ))
    });
    fs::create_dir_all(&output).map_err(message)?;
    let name = if directory {
        format!("{} · {} suites", path.display(), suites.len())
    } else {
        suites[0].1.name.clone()
    };
    let mut viewer = if ui {
        let logs = output.join("viewer");
        fs::create_dir_all(&logs).map_err(message)?;
        let mut command = Command::new(std::env::current_exe().map_err(message)?);
        command.arg("__test-results");
        if background {
            command.arg("--background");
        }
        Some(launch_command(command, &logs, Duration::from_secs(10))?)
    } else {
        None
    };
    let mut previous: Vec<SuiteReport> = Vec::new();
    let mut only_failed = false;
    let mut status;
    loop {
        let started = Instant::now();
        let client = viewer.as_ref().map(|(_, client)| client);
        if let Some(client) = client {
            let mut offset = 0;
            for (suite_index, (source, suite)) in suites.iter().enumerate() {
                for chunk in suite
                    .tests
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| {
                        !only_failed
                            || previous
                                .get(suite_index)
                                .is_none_or(|report| report.tests[*index].report.status == "failed")
                    })
                    .collect::<Vec<_>>()
                    .chunks(16)
                {
                    progress(client,&name,chunk.iter().map(|(index,case)| json!({"index":offset+index,"suite":source.display().to_string(),"name":case.name,"status":"pending"})).collect())?;
                }
                offset += suite.tests.len();
            }
        }
        let mut reports = Vec::new();
        let mut offset = 0;
        for (index, (source, suite)) in suites.iter().enumerate() {
            let suite_output = if directory {
                output.join(format!("suite-{:03}", index + 1))
            } else {
                output.clone()
            };
            let report = run_suite(
                source,
                suite_output,
                suite.clone(),
                client,
                &name,
                offset,
                previous.get(index),
                only_failed,
            )?;
            offset += suite.tests.len();
            reports.push(report);
        }
        status = if reports.iter().any(|report| report.status == "failed") {
            "failed"
        } else {
            "passed"
        };
        if directory {
            let tests = reports
                .iter()
                .enumerate()
                .flat_map(|(index, report)| {
                    report.tests.iter().cloned().map(move |mut case| {
                        case.directory = format!("suite-{:03}/{}", index + 1, case.directory);
                        case
                    })
                })
                .collect();
            write_suite_report(
                &output,
                &SuiteReport {
                    schema_version: 1,
                    name: name.clone(),
                    status,
                    duration_ms: started.elapsed().as_millis(),
                    startup_duration_ms: reports
                        .iter()
                        .map(|report| report.startup_duration_ms)
                        .sum(),
                    tests,
                },
            )?;
        }
        previous = reports;
        let Some((app, client)) = &mut viewer else {
            break;
        };
        println!("Close the Test Results window to exit, or use Run all / Rerun failed.");
        loop {
            if app.child.try_wait().map_err(message)?.is_some() {
                break;
            }
            let commands = match client.call(
                json!({"method":"testCommands"}),
                Instant::now() + Duration::from_secs(5),
            ) {
                Ok(commands) => commands,
                Err(error) => {
                    thread::sleep(Duration::from_millis(100));
                    if app.child.try_wait().map_err(message)?.is_some() {
                        break;
                    }
                    return Err(error);
                }
            };
            if let Some(command) = commands
                .as_array()
                .and_then(|commands| commands.last())
                .and_then(Value::as_str)
            {
                only_failed = command == "failed";
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
        if app.child.try_wait().map_err(message)?.is_some() {
            break;
        }
    }
    if status == "failed" {
        Err("UI suite failed".into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directory_discovery_is_recursive_sorted_and_ignores_non_scenario_json() {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir(directory.path().join("nested")).unwrap();
        let scenario = json!({"schemaVersion":1,"name":"Example","app":"app.lua","steps":[{"action":"screenshot"}]});
        fs::write(
            directory.path().join("z.json"),
            serde_json::to_vec(&scenario).unwrap(),
        )
        .unwrap();
        fs::write(
            directory.path().join("nested/a.json"),
            serde_json::to_vec(&scenario).unwrap(),
        )
        .unwrap();
        fs::write(
            directory.path().join("tsconfig.json"),
            r#"{"compilerOptions":{}}"#,
        )
        .unwrap();
        fs::write(directory.path().join("app.lua"), "return function() end").unwrap();
        let found = discover(directory.path()).unwrap();
        assert_eq!(found.len(), 2);
        assert!(found[0].0.ends_with("nested/a.json"));
        assert!(found[1].0.ends_with("z.json"));
        assert_eq!(found[0].1.app, PathBuf::from("app.lua"));
        fs::write(directory.path().join("broken.json"), "not json").unwrap();
        assert!(discover(directory.path())
            .unwrap_err()
            .contains("broken.json"));
    }

    #[test]
    fn directory_discovery_rejects_empty_directories_and_invalid_scenarios() {
        let directory = tempfile::tempdir().unwrap();
        assert!(discover(directory.path()).is_err());
        fs::write(directory.path().join("bad.json"),r#"{"schemaVersion":2,"name":"Bad","app":"app.lua","tests":[{"name":"Bad","steps":[{"action":"screenshot"}]}]}"#).unwrap();
        assert!(discover(directory.path()).is_err());
    }

    #[test]
    fn ui_cases_capture_each_executed_step_without_changing_non_ui_runs() {
        for screenshots in [false, true] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let address = listener.local_addr().unwrap();
            let server = std::thread::spawn(move || {
                let expected = if screenshots { 6 } else { 2 };
                let mut methods = Vec::new();
                let deadline = Instant::now() + Duration::from_secs(5);
                while methods.len() < expected && Instant::now() < deadline {
                    let (mut stream, _) = match listener.accept() {
                        Ok(stream) => stream,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(1));
                            continue;
                        }
                        Err(error) => panic!("{error}"),
                    };
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    let mut reader = BufReader::new(&mut stream);
                    let mut length = 0;
                    loop {
                        let mut line = String::new();
                        reader.read_line(&mut line).unwrap();
                        if line == "\r\n" {
                            break;
                        }
                        if let Some(value) = line.strip_prefix("Content-Length: ") {
                            length = value.trim().parse().unwrap();
                        }
                    }
                    let mut body = vec![0; length];
                    reader.read_exact(&mut body).unwrap();
                    let command: Value = serde_json::from_slice(&body).unwrap();
                    let method = command["method"].as_str().unwrap().to_string();
                    let value = if method == "screenshot" {
                        json!({"data":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jB9kAAAAASUVORK5CYII="})
                    } else {
                        json!({})
                    };
                    methods.push(method);
                    let body = json!({"result":value}).to_string();
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .unwrap();
                }
                methods
            });
            let client = Client {
                address,
                token: "test-token".into(),
            };
            let directory = tempfile::tempdir().unwrap();
            let scenario = Scenario {
                schema_version: 1,
                name: "Evidence".into(),
                app: "unused.lua".into(),
                timeout_ms: 1000,
                steps: vec![
                    Step {
                        name: None,
                        timeout_ms: None,
                        action: Action::Press {
                            keys: "enter".into()
                        }
                    };
                    2
                ],
            };
            let report = execute_case(&scenario, &client, directory.path(), Ok(1), screenshots);
            assert_eq!(report.status, "passed");
            assert_eq!(
                report
                    .steps
                    .iter()
                    .filter(|step| step.screenshot.is_some())
                    .count(),
                if screenshots { 2 } else { 0 }
            );
            let methods = server.join().unwrap();
            assert_eq!(
                methods
                    .iter()
                    .filter(|method| *method == "screenshot")
                    .count(),
                if screenshots { 2 } else { 0 }
            );
            if screenshots {
                assert!(directory.path().join("step-001.png").exists());
                assert!(directory.path().join("step-002.png").exists());
                write_report(directory.path(), &report).unwrap();
                assert!(fs::read_to_string(directory.path().join("report.html"))
                    .unwrap()
                    .contains("step-002.png"));
            }
        }
    }

    #[test]
    fn suite_cases_use_the_shared_application_and_allow_timeout_overrides() {
        let suite: Suite = serde_json::from_value(json!({
            "schemaVersion":1, "name":"Suite", "app":"counter.luax", "tests":[
                {"name":"First", "steps":[{"action":"click", "testId":"counter"}]},
                {"name":"Second", "timeoutMs":100, "steps":[{"action":"expectText", "testId":"counter", "text":"0"}]}
            ]
        })).unwrap();
        assert_eq!(suite.tests.len(), 2);
        assert_eq!(suite.tests[0].timeout_ms.unwrap_or(suite.timeout_ms), 5000);
        assert_eq!(suite.tests[1].timeout_ms, Some(100));
        assert!(serde_json::from_value::<Suite>(
            json!({"schemaVersion":1,"name":"Suite","app":"counter.lua","tests":[],"reset":false})
        )
        .is_err());
    }

    #[test]
    fn parses_steps_and_rejects_misspelled_fields() {
        let value = json!({"schemaVersion":1, "name":"Counter", "app":"counter.luax", "steps":[
            {"action":"expectText", "testId":"count", "text":"0", "exact":true, "name":"initial", "timeoutMs":100},
            {"action":"click", "testId":"increment"},
            {"action":"press", "keys":"tab enter"},
            {"action":"expectVisible", "testId":"count"},
            {"action":"expectAbsent", "testId":"gone"},
            {"action":"expectProp", "testId":"input", "prop":"value", "value":"hello"},
            {"action":"window", "index":1}, {"action":"screenshot"}
        ]});
        let scenario: Scenario = serde_json::from_value(value.clone()).unwrap();
        validate(&scenario).unwrap();
        assert_eq!(scenario.steps.len(), 8);
        let mut invalid = value.clone();
        invalid["steps"][1]["testID"] = json!("wrong");
        assert!(serde_json::from_value::<Scenario>(invalid).is_err());
        let mut invalid = value.clone();
        invalid["steps"][0]["timeoutMs"] = json!(0);
        assert!(validate(&serde_json::from_value(invalid).unwrap()).is_err());
        let mut invalid = value.clone();
        invalid["schemaVersion"] = json!(2);
        assert!(validate(&serde_json::from_value(invalid).unwrap()).is_err());
        let mut invalid = value;
        invalid["steps"] = json!([]);
        assert!(validate(&serde_json::from_value(invalid).unwrap()).is_err());
    }

    #[test]
    fn selectors_are_strict_and_text_merges_descendants() {
        let tree = json!({"id":1,"children":[{"id":2,"testId":"count","children":[{"text":"Count: "},{"text":"1"}]}]});
        assert_eq!(text(unique(&tree, "count").unwrap()), "Count: 1");
        assert!(unique(&tree, "missing").unwrap_err().contains("found 0"));
        let duplicate = json!({"children":[{"testId":"same"},{"testId":"same"}]});
        assert!(unique(&duplicate, "same").unwrap_err().contains("found 2"));
    }

    #[test]
    fn polling_retries_and_preserves_final_failure() {
        let mut calls = 0;
        let value = retry(Instant::now() + Duration::from_secs(1), || {
            calls += 1;
            if calls == 2 {
                Ok(42)
            } else {
                Err("not ready".into())
            }
        })
        .unwrap();
        assert_eq!(value, 42);
        let error =
            retry::<()>(Instant::now(), || Err("expected count 2, got 1".into())).unwrap_err();
        assert!(error.contains("got 1"));
    }

    #[test]
    fn http_client_authenticates_and_reports_api_errors() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let task = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut headers = String::new();
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
                headers.push_str(&line);
            }
            assert!(headers.contains("Authorization: Bearer example-token"));
            let length = headers
                .lines()
                .find_map(|line| line.strip_prefix("Content-Length: "))
                .unwrap()
                .trim()
                .parse::<usize>()
                .unwrap();
            let mut request_body = vec![0; length];
            reader.read_exact(&mut request_body).unwrap();
            let body = r#"{"error":"window not found"}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });
        let client = Client {
            address,
            token: "example-token".into(),
        };
        assert_eq!(
            client
                .call(
                    json!({"method":"windows"}),
                    Instant::now() + Duration::from_secs(2)
                )
                .unwrap_err(),
            "window not found"
        );
        task.join().unwrap();
    }

    #[test]
    fn html_escapes_application_values_and_links_failure_artifacts() {
        let directory = tempfile::tempdir().unwrap();
        let report = Report {
            schema_version: 1,
            name: "<script>alert('app')</script>".into(),
            status: "failed",
            duration_ms: 3,
            error: None,
            steps: vec![StepReport {
                step: Step {
                    name: Some("<b>unsafe</b>".into()),
                    timeout_ms: None,
                    action: Action::Screenshot {},
                },
                status: "failed",
                duration_ms: 2,
                error: Some("<img src=x onerror=alert(1)>".into()),
                screenshot: Some("step-001.png".into()),
                tree: Some("step-001.tree.json".into()),
                artifact_errors: vec![],
                snapshot: None,
            }],
        };
        write_report(directory.path(), &report).unwrap();
        let html = fs::read_to_string(directory.path().join("report.html")).unwrap();
        assert!(!html.contains("<script>"));
        assert!(!html.contains("<img src=x"));
        assert!(html.contains("&lt;b&gt;unsafe"));
        assert!(html.contains("step-001.tree.json"));
        let json: Value =
            serde_json::from_slice(&fs::read(directory.path().join("report.json")).unwrap())
                .unwrap();
        assert_eq!(json["steps"][0]["status"], "failed");
    }
}
