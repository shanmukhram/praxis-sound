//! End to end through the real `praxis-mcp` binary over stdio (newline-delimited JSON-RPC, the
//! MCP stdio transport): initialize, list the tools, and call every tool's happy path and one
//! error path. The acceptance-1 plan goes through `praxis_compose` with a render.
//!
//! Rendering needs the Upright Piano KW pack (the only renderable instrument so far). The test
//! uses `$PRAXIS_PACKS`, else the first `packs/upright-piano-kw` found above this crate (the
//! main checkout's `packs/` when run from a worktree). Without it the render steps are skipped
//! with a printed message and the rest still runs.

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(300);

struct Client {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
    next_id: u64,
    progress: Vec<Value>,
}

impl Client {
    fn spawn(packs: Option<&Path>, cwd: &Path) -> Client {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_praxis-mcp"));
        cmd.current_dir(cwd)
            .env("PRAXIS_KEYCHAIN", "off")
            .env_remove("ANTHROPIC_API_KEY")
            .env_remove("PRAXIS_CLAUDE_URL")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        match packs {
            Some(p) => cmd.env("PRAXIS_PACKS", p),
            // A folder with no packs, so a stray ./packs is not picked up.
            None => cmd.env("PRAXIS_PACKS", cwd.join("no-packs")),
        };
        let mut child = cmd.spawn().expect("spawn praxis-mcp");
        let stdin = child.stdin.take().expect("stdin");
        let stdout = child.stdout.take().expect("stdout");
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        Client {
            child,
            stdin,
            lines: rx,
            next_id: 1,
            progress: Vec::new(),
        }
    }

    fn send(&mut self, msg: &Value) {
        let mut text = serde_json::to_string(msg).unwrap();
        assert!(!text.contains('\n'));
        text.push('\n');
        self.stdin.write_all(text.as_bytes()).unwrap();
        self.stdin.flush().unwrap();
    }

    /// Sends a request and waits for its response, collecting progress notifications.
    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        loop {
            let line = self
                .lines
                .recv_timeout(TIMEOUT)
                .unwrap_or_else(|_| panic!("no response to {method} within {TIMEOUT:?}"));
            let msg: Value = serde_json::from_str(&line)
                .unwrap_or_else(|e| panic!("stdout carried a non-JSON line ({e}): {line}"));
            if msg.get("id") == Some(&json!(id)) {
                return msg;
            }
            if msg.get("method") == Some(&json!("notifications/progress")) {
                self.progress.push(msg);
            }
        }
    }

    /// Calls a tool; returns (isError, structuredContent).
    fn call(&mut self, name: &str, args: Value) -> (bool, Value) {
        self.call_with_progress(name, args, None)
    }

    fn call_with_progress(
        &mut self,
        name: &str,
        args: Value,
        token: Option<&str>,
    ) -> (bool, Value) {
        let mut params = json!({"name": name, "arguments": args});
        if let Some(t) = token {
            params["_meta"] = json!({"progressToken": t});
        }
        let resp = self.request("tools/call", params);
        assert!(resp.get("error").is_none(), "{name}: protocol error {resp}");
        let result = &resp["result"];
        let is_error = result["isError"].as_bool().unwrap_or(false);
        let body = result["structuredContent"].clone();
        assert!(
            body.is_object(),
            "{name}: structured content missing: {resp}"
        );
        // The text content mirrors the structured content (for clients that ignore the latter).
        let text = result["content"][0]["text"].as_str().expect("text content");
        assert_eq!(serde_json::from_str::<Value>(text).unwrap(), body);
        assert_eq!(body["ok"], json!(!is_error), "{name}: ok flag vs isError");
        (is_error, body)
    }

    /// Calls a tool expecting success.
    fn ok(&mut self, name: &str, args: Value) -> Value {
        let (is_error, body) = self.call(name, args);
        assert!(!is_error, "{name} failed: {body:#}");
        body
    }

    /// Calls a tool expecting a failure with `code`.
    fn err(&mut self, name: &str, args: Value, code: &str) -> Value {
        let (is_error, body) = self.call(name, args);
        assert!(is_error, "{name} should fail with {code}: {body:#}");
        assert_eq!(body["error"]["code"], json!(code), "{name}: {body:#}");
        assert!(
            body["error"]["message"]
                .as_str()
                .is_some_and(|m| !m.is_empty())
        );
        body
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn find_packs() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("PRAXIS_PACKS") {
        let p = PathBuf::from(p);
        return p.join("upright-piano-kw").is_dir().then_some(p);
    }
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .map(|d| d.join("packs"))
        .find(|p| p.join("upright-piano-kw").is_dir())
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("praxis-mcp-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::path::absolute(dir).unwrap()
}

/// `dir/rel` as the server reports it: absolute, with native separators.
fn at(dir: &Path, rel: &str) -> PathBuf {
    std::path::absolute(dir.join(rel)).unwrap()
}

fn s(p: &Path) -> String {
    p.display().to_string()
}

fn initialize(c: &mut Client) -> Value {
    let init = c.request(
        "initialize",
        json!({
            "protocolVersion": "2025-11-25",
            "capabilities": {},
            "clientInfo": {"name": "praxis-mcp-test", "version": "0"},
        }),
    );
    c.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
    init
}

const TOOLS: [&str; 9] = [
    "praxis_analyze",
    "praxis_check",
    "praxis_compose",
    "praxis_export",
    "praxis_generate",
    "praxis_plan_schema",
    "praxis_presets",
    "praxis_render",
    "praxis_validate",
];

#[test]
fn every_tool_over_stdio() {
    let dir = temp_dir("tools");
    let packs = find_packs();
    let mut c = Client::spawn(packs.as_deref(), &dir);

    // --- initialize and list ---
    let init = initialize(&mut c);
    let r = &init["result"];
    assert_eq!(r["serverInfo"]["name"], "praxis-mcp", "{init}");
    assert!(r["capabilities"]["tools"].is_object(), "{init}");
    assert!(
        r["instructions"]
            .as_str()
            .is_some_and(|i| i.contains("praxis_compose"))
    );
    let list = c.request("tools/list", json!({}));
    let tools = list["result"]["tools"].as_array().expect("tools").clone();
    let mut names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    names.sort();
    assert_eq!(names, TOOLS);
    for t in &tools {
        assert_eq!(t["inputSchema"]["type"], "object", "{t}");
        assert!(
            t["description"].as_str().is_some_and(|d| d.len() > 40),
            "{t}"
        );
    }
    let compose_schema = &tools
        .iter()
        .find(|t| t["name"] == "praxis_compose")
        .unwrap()["inputSchema"];
    let required: Vec<&str> = compose_schema["required"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(required.contains(&"plan") && required.contains(&"project"));
    for f in [
        "render",
        "seed",
        "format",
        "sample_rate",
        "stems",
        "skip_missing",
    ] {
        assert!(compose_schema["properties"][f].is_object(), "compose.{f}");
    }

    // --- praxis_plan_schema ---
    let schema = c.ok("praxis_plan_schema", json!({}));
    assert_eq!(schema["command"], "schema");
    assert!(
        schema["plan"]["properties"]["layers"].is_object(),
        "{schema:#}"
    );
    assert!(schema["how_to"].as_array().is_some_and(|h| h.len() >= 5));
    assert_eq!(schema["example"]["key"]["tonic"], "A");
    assert!(schema.get("score").is_none());
    let full = c.ok("praxis_plan_schema", json!({"include_score": true}));
    assert!(full["score"].is_object() && full["patch"].is_object());
    let e = c.err(
        "praxis_plan_schema",
        json!({"include_scores": true}),
        "bad_json",
    );
    assert_eq!(e["error"]["issues"][0]["path"], "/include_scores");

    // --- praxis_presets ---
    let presets = c.ok("praxis_presets", json!({}));
    let list = presets["presets"].as_array().unwrap();
    assert!(list.iter().any(|p| p["name"] == "felt_piano"));
    let piano = list.iter().find(|p| p["name"] == "felt_piano").unwrap();
    assert_eq!(piano["available"], json!(packs.is_some()), "{piano}");
    c.err("praxis_presets", json!({"verbose": true}), "bad_json");

    // --- praxis_compose (no render) ---
    let plan: Value = serde_json::from_str(
        &std::fs::read_to_string(
            workspace().join("crates/praxis-planner/tests/prompts/acceptance1.plan.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let project = at(&dir, "plain/loop.praxis");
    let composed = c.ok(
        "praxis_compose",
        json!({"plan": plan, "project": s(&project)}),
    );
    assert_eq!(composed["command"], "generate");
    assert_eq!(composed["project"], json!(s(&project)));
    assert!(project.is_file());
    assert_eq!(composed["score_summary"]["bars"], 16);
    assert_eq!(composed["score_summary"]["bpm"], 128.0);
    assert_eq!(composed["score_summary"]["key"], "a minor");
    assert!(composed.get("render").is_none());
    // Relative paths resolve against the server's working directory and come back absolute.
    let rel = c.ok(
        "praxis_compose",
        json!({"plan": plan, "project": "rel/loop.praxis"}),
    );
    assert_eq!(rel["project"], json!(s(&at(&dir, "rel/loop.praxis"))));
    // Error: an unrepairable plan; issues point into the plan.
    let mut bad = plan.clone();
    bad["key"]["tonic"] = json!("H");
    let e = c.err(
        "praxis_compose",
        json!({"plan": bad, "project": s(&at(&dir, "bad.praxis"))}),
        "invalid_score",
    );
    assert_eq!(e["error"]["issues"][0]["path"], "/key", "{e:#}");
    assert!(
        e["error"]["issues"][0]["message"]
            .as_str()
            .unwrap()
            .contains("`A`")
    );
    c.err(
        "praxis_compose",
        json!({"project": s(&at(&dir, "x.praxis"))}),
        "bad_json",
    );

    // --- praxis_generate (offline brief) ---
    let gen_project = at(&dir, "gen/brief.praxis");
    let generated = c.ok(
        "praxis_generate",
        json!({
            "prompt": "8 bars, D minor, 90 BPM, seamless loop, a soft felt piano motif on an eighth grid",
            "project": s(&gen_project),
            "provider": "offline",
        }),
    );
    assert_eq!(generated["provider"], "offline-rules", "{generated:#}");
    assert_eq!(generated["score_summary"]["bars"], 8);
    assert!(gen_project.is_file());
    // Error: the project path is under a regular file.
    let blocker = at(&dir, "blocker.txt");
    std::fs::write(&blocker, "not a folder").unwrap();
    c.err(
        "praxis_generate",
        json!({"prompt": "4 bars, C major, felt piano", "project": s(&blocker.join("x.praxis")), "provider": "offline"}),
        "io",
    );

    // --- praxis_validate ---
    let v = c.ok(
        "praxis_validate",
        json!({"input": s(&workspace().join("fixtures/acceptance1.score.json"))}),
    );
    assert_eq!(v["valid"], true);
    c.err(
        "praxis_validate",
        json!({"input": s(&project)}),
        "bad_score",
    );

    // --- praxis_export (MIDI needs no instruments) ---
    let mid = at(&dir, "out/loop.mid");
    let m = c.ok(
        "praxis_export",
        json!({"input": s(&project), "out": s(&mid), "format": "midi"}),
    );
    assert_eq!(m["command"], "export-midi");
    assert_eq!(m["tracks"], 3);
    assert!(std::fs::metadata(&mid).unwrap().len() > 0);
    c.err(
        "praxis_export",
        json!({"input": s(&project), "out": s(&at(&dir, "x.mp3")), "format": "mp3"}),
        "bad_format",
    );

    // --- error paths that need no audio ---
    c.err(
        "praxis_render",
        json!({"input": s(&at(&dir, "missing.praxis")), "out": s(&at(&dir, "m.wav"))}),
        "io",
    );
    c.err(
        "praxis_check",
        json!({"audio": s(&at(&dir, "missing.wav")), "score": s(&project)}),
        "io",
    );
    c.err(
        "praxis_analyze",
        json!({"clip": s(&project)}),
        "decode_failed",
    );

    let Some(_) = packs else {
        eprintln!(
            "SKIP render steps: the Upright Piano KW pack was not found (set PRAXIS_PACKS to a \
             folder containing upright-piano-kw)"
        );
        // Without any renderable instrument, a render fails with a coded error.
        c.err(
            "praxis_render",
            json!({"input": s(&project), "out": s(&at(&dir, "r.wav")), "skip_missing": true}),
            "render_failed",
        );
        return;
    };

    // --- praxis_compose with a render: the acceptance-1 loop ---
    let project = at(&dir, "acceptance1/loop.praxis");
    let wav = at(&dir, "acceptance1/loop.wav");
    let (is_error, body) = c.call_with_progress(
        "praxis_compose",
        json!({"plan": plan, "project": s(&project), "render": s(&wav), "skip_missing": true}),
        Some("compose-1"),
    );
    assert!(!is_error, "{body:#}");
    assert_eq!(body["project"], json!(s(&project)));
    assert!(project.is_file());
    let render = &body["render"];
    assert_eq!(render["output"], json!(s(&wav)));
    assert!(wav.is_file());
    assert_eq!(render["frames"], 30 * 48_000);
    assert_eq!(render["is_loop"], true);
    let check = &render["check"];
    let checks = check["checks"].as_array().expect("verify report checks");
    for ch in checks {
        eprintln!(
            "check {:<10} {:<5} measured {} | expected {}",
            ch["name"].as_str().unwrap_or("?"),
            if ch["passed"] == true { "PASS" } else { "FAIL" },
            ch["measured"],
            ch["expected"]
        );
    }
    let names: Vec<&str> = checks.iter().filter_map(|c| c["name"].as_str()).collect();
    for n in [
        "tempo",
        "key",
        "grid",
        "loudness",
        "true_peak",
        "clipping",
        "loop_seam",
        "silence",
    ] {
        assert!(names.contains(&n), "check {n} missing from {names:?}");
    }
    assert_eq!(
        check["passed"], true,
        "every check passes: failed {}",
        check["failed"]
    );
    for sk in render["skipped_layers"].as_array().unwrap() {
        eprintln!("NOTE: layer skipped: {sk}");
    }
    assert_eq!(
        c.progress
            .iter()
            .filter(|p| p["params"]["progressToken"] == "compose-1")
            .count(),
        2,
        "start and finish progress notifications: {:?}",
        c.progress
    );

    // --- praxis_render ---
    let flac = at(&dir, "render/loop.flac");
    let stems = at(&dir, "render/stems");
    let r = c.ok(
        "praxis_render",
        json!({"input": s(&project), "out": s(&flac), "format": "flac", "stems": s(&stems), "skip_missing": true}),
    );
    assert_eq!(r["format"], "flac24");
    assert!(flac.is_file());
    assert!(!r["stems"].as_array().unwrap().is_empty());
    assert!(r["check"]["passed"].is_boolean());
    c.err(
        "praxis_render",
        json!({"input": s(&project), "out": s(&at(&dir, "r.wav")), "format": "mp3"}),
        "bad_format",
    );

    // --- praxis_check ---
    let ck = c.ok(
        "praxis_check",
        json!({"audio": s(&wav), "score": s(&project)}),
    );
    assert_eq!(ck["command"], "check");
    assert!(ck["checks"].is_array() && ck["passed"].is_boolean());

    // --- praxis_analyze ---
    let a = c.ok("praxis_analyze", json!({"clip": s(&wav)}));
    assert!(
        (a["source"]["duration_s"].as_f64().unwrap() - 30.0).abs() < 0.01,
        "{a:#}"
    );
    assert!(a["tempo"].is_object() && a["loudness"].is_object());

    // --- praxis_export (audio) ---
    let w16 = at(&dir, "export/loop16.wav");
    let x = c.ok(
        "praxis_export",
        json!({"input": s(&project), "out": s(&w16), "format": "16", "skip_missing": true}),
    );
    assert_eq!(x["format"], "wav16");
    assert!(w16.is_file() && x["check"]["checks"].is_array());

    // Nothing but protocol messages reached stdout (every line parsed as JSON above), and no
    // response carried audio bytes.
    drop(c);
    let _ = std::fs::remove_dir_all(&dir);
}
