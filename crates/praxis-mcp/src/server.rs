//! The MCP tool router. Every tool parses its arguments (see [`crate::args`]), runs the matching
//! `praxis_cli::ops` function on a blocking thread, and returns the CLI's JSON envelope
//! (`{"ok", "command", ...payload}` or `{"ok": false, "command", "error": {code, message,
//! issues?}}`) as structured content. Failures set `isError` so the calling agent sees them.

use crate::args::{
    AnalyzeArgs, AskArgs, CheckArgs, ComposeArgs, EditArgs, Export, ExportArgs, GenerateArgs,
    KeepVariationArgs, NoArgs, OpenViewArgs, PlanSchemaArgs, RenderArgs, RestoreArgs, TakesArgs,
    ValidateArgs, VariationsArgs, parse,
};
use crate::guide;
use praxis_cli::ops;
use praxis_cli::{ErrorCode, OpError, envelope};
use rmcp::handler::server::common::schema_for_input;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::model::{
    CallToolResult, Implementation, JsonObject, ProgressNotificationParam, RequestMetaObject,
    ServerCapabilities, ServerConfig,
};
use rmcp::{ErrorData, Peer, RoleServer, ServerHandler, tool, tool_handler, tool_router};
use serde_json::{Value, json};

/// The Praxis Sound MCP server.
#[derive(Debug, Clone)]
pub struct PraxisServer {
    tool_router: ToolRouter<Self>,
}

impl Default for PraxisServer {
    fn default() -> Self {
        Self::new()
    }
}

/// The schema of `T`, for a `#[tool(input_schema = ...)]` attribute. The argument types are
/// plain structs, so schema generation cannot fail; if it ever did, the tool advertises an
/// empty object schema instead of panicking.
fn input_schema<T: schemars::JsonSchema + 'static>() -> std::sync::Arc<JsonObject> {
    schema_for_input::<T>()
        .unwrap_or_else(|_| rmcp::handler::server::common::schema_for_empty_input())
}

/// Wraps a command result in the CLI envelope.
pub fn tool_result(command: &str, result: Result<Value, OpError>) -> CallToolResult {
    let body = envelope(command, &result);
    match result {
        Ok(_) => CallToolResult::structured(body),
        Err(_) => CallToolResult::structured_error(body),
    }
}

/// Runs a (possibly long, CPU-bound) command off the async runtime. When the caller sent a
/// progress token, reports a start and a finish notification around it.
async fn run<F>(
    command: &'static str,
    meta: &RequestMetaObject,
    peer: &Peer<RoleServer>,
    work: F,
) -> CallToolResult
where
    F: FnOnce() -> Result<Value, OpError> + Send + 'static,
{
    let token = meta.get_progress_token();
    if let Some(t) = &token {
        let _ = peer
            .notify_progress(
                ProgressNotificationParam::new(t.clone(), 0.0)
                    .with_total(1.0)
                    .with_message(format!("{command}: started")),
            )
            .await;
    }
    let result = match tokio::task::spawn_blocking(work).await {
        Ok(r) => r,
        Err(e) => Err(OpError::new(
            ErrorCode::Error,
            format!("{command} stopped unexpectedly: {e}"),
        )),
    };
    if let Some(t) = &token {
        let _ = peer
            .notify_progress(
                ProgressNotificationParam::new(t.clone(), 1.0)
                    .with_total(1.0)
                    .with_message(format!("{command}: finished")),
            )
            .await;
    }
    tool_result(command, result)
}

/// Who a take is from when the agent does not say: `"Agent · <MCP client name>"`.
pub fn client_who(peer: &Peer<RoleServer>) -> String {
    let name = peer
        .peer_info()
        .map(|i| i.client_info.name.trim().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "mcp".to_string());
    format!("Agent \u{b7} {name}")
}

/// The `praxis_plan_schema` payload: the ScorePlan schema, a how-to and a complete example.
pub fn plan_schema_payload(args: &PlanSchemaArgs) -> Result<Value, OpError> {
    let schemas = ops::schema(&ops::SchemaRequest { write: false })?;
    let example: Value = serde_json::from_str(guide::EXAMPLE_PLAN)
        .map_err(|e| OpError::new(ErrorCode::Error, format!("example plan: {e}")))?;
    let mut v = json!({
        "plan": schemas["plan"],
        "how_to": guide::PLAN_HOW_TO,
        "example": example,
        "spoken_tokens": {
            "DON": "strong hit (accent)",
            "don": "normal hit",
            "din": "soft hit (ghost)",
            "TIN": "bright hit",
            "-": "rest",
            "~": "tie (extends the previous note)",
        },
        "next": "Call praxis_presets for preset names, then praxis_compose with your plan, a project path and a render path.",
    });
    if args.include_score {
        v["score"] = schemas["score"].clone();
        v["patch"] = schemas["patch"].clone();
    }
    Ok(v)
}

#[tool_router]
impl PraxisServer {
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        name = "praxis_plan_schema",
        description = "The ScorePlan JSON Schema you fill to compose a sound yourself (no API key needed), with a short how-to, the spoken-rhythm tokens and a complete example plan. Start here.",
        input_schema = input_schema::<PlanSchemaArgs>(),
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn plan_schema(&self, args: JsonObject) -> Result<CallToolResult, ErrorData> {
        let result = parse::<PlanSchemaArgs>(args).and_then(|a| plan_schema_payload(&a));
        Ok(tool_result("schema", result))
    }

    #[tool(
        name = "praxis_presets",
        description = "The presets a ScorePlan may name (name, kind, class, pitched, sustaining, range, description) and whether each renders on this machine (`available`, else `reason`).",
        input_schema = input_schema::<NoArgs>(),
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn presets(
        &self,
        args: JsonObject,
        meta: RequestMetaObject,
        peer: Peer<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        if let Err(e) = parse::<NoArgs>(args) {
            return Ok(tool_result("presets", Err(e)));
        }
        Ok(run("presets", &meta, &peer, ops::presets).await)
    }

    #[tool(
        name = "praxis_compose",
        description = "Turn a ScorePlan you wrote into a .praxis project: validated, deterministically repaired (each change listed in `repairs`) and arranged. With `render`, also renders the master and returns measurements and checks under `render.check` (passed, failed[], tempo, key, grid, loudness, true peak, clipping, loop seam). No API key needed. Invalid plans fail with invalid_score and issues whose `path` is a JSON pointer into your plan.",
        input_schema = input_schema::<ComposeArgs>(),
        annotations(read_only_hint = false, destructive_hint = false, open_world_hint = false)
    )]
    async fn compose(
        &self,
        args: JsonObject,
        meta: RequestMetaObject,
        peer: Peer<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let req = match parse::<ComposeArgs>(args) {
            Ok(a) => a.request(),
            Err(e) => return Ok(tool_result("generate", Err(e))),
        };
        Ok(run("generate", &meta, &peer, move || ops::generate(&req)).await)
    }

    #[tool(
        name = "praxis_generate",
        description = "Turn a plain-language brief into a .praxis project through the offline rule planner, or Claude when an API key is available (`provider` says which was used). With `render`, also renders and returns the checks. To control every detail, write the plan yourself with praxis_compose instead.",
        input_schema = input_schema::<GenerateArgs>(),
        annotations(read_only_hint = false, destructive_hint = false, open_world_hint = true)
    )]
    async fn generate(
        &self,
        args: JsonObject,
        meta: RequestMetaObject,
        peer: Peer<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let req = match parse::<GenerateArgs>(args) {
            Ok(a) => a.request(),
            Err(e) => return Ok(tool_result("generate", Err(e))),
        };
        Ok(run("generate", &meta, &peer, move || ops::generate(&req)).await)
    }

    #[tool(
        name = "praxis_render",
        description = "Render a .praxis project or .score.json to an audio file (WAV or FLAC, optional stems). Returns the output path, duration, loudness, true peak, skipped layers, warnings and the checks under `check`.",
        input_schema = input_schema::<RenderArgs>(),
        annotations(read_only_hint = false, destructive_hint = false, open_world_hint = false)
    )]
    async fn render(
        &self,
        args: JsonObject,
        meta: RequestMetaObject,
        peer: Peer<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let req = match parse::<RenderArgs>(args) {
            Ok(a) => a.request(),
            Err(e) => return Ok(tool_result("render", Err(e))),
        };
        Ok(run("render", &meta, &peer, move || ops::render(&req)).await)
    }

    #[tool(
        name = "praxis_check",
        description = "Measure an existing render against its project or score: detected tempo and key, grid alignment, integrated loudness, true peak, clipping, loop seam and silence, each with measured and expected values, plus `passed` and `failed[]`.",
        input_schema = input_schema::<CheckArgs>(),
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn check(
        &self,
        args: JsonObject,
        meta: RequestMetaObject,
        peer: Peer<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let req = match parse::<CheckArgs>(args) {
            Ok(a) => a.request(),
            Err(e) => return Ok(tool_result("check", Err(e))),
        };
        Ok(run("check", &meta, &peer, move || ops::check(&req)).await)
    }

    #[tool(
        name = "praxis_analyze",
        description = "Measure an audio or video clip: duration, tempo and beats, key with confidence, loudness (integrated, range, true peak), onsets and loop seam. `complete: false` lists Sound Profile fields not measured yet.",
        input_schema = input_schema::<AnalyzeArgs>(),
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn analyze(
        &self,
        args: JsonObject,
        meta: RequestMetaObject,
        peer: Peer<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let req = match parse::<AnalyzeArgs>(args) {
            Ok(a) => a.request(),
            Err(e) => return Ok(tool_result("analyze", Err(e))),
        };
        Ok(run("analyze", &meta, &peer, move || ops::analyze(&req)).await)
    }

    #[tool(
        name = "praxis_export",
        description = "Export a .praxis project or .score.json as WAV (16/24/32f), FLAC (16/24), per-layer stems (`stems` directory) or a Standard MIDI File (`format: \"midi\"`). Audio exports return the render report and checks; MIDI returns the tracks written.",
        input_schema = input_schema::<ExportArgs>(),
        annotations(read_only_hint = false, destructive_hint = false, open_world_hint = false)
    )]
    async fn export(
        &self,
        args: JsonObject,
        meta: RequestMetaObject,
        peer: Peer<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let export = match parse::<ExportArgs>(args).and_then(|a| a.request()) {
            Ok(x) => x,
            Err(e) => return Ok(tool_result("render", Err(e))),
        };
        Ok(match export {
            Export::Audio(req) => run("render", &meta, &peer, move || ops::render(&req)).await,
            Export::Midi(req) => {
                run("export-midi", &meta, &peer, move || ops::export_midi(&req)).await
            }
        })
    }

    #[tool(
        name = "praxis_edit",
        description = "Apply a scoped edit to a .praxis project (FR-7): `prompt` in words (\"change the last four bars to add a subtle melody\") or an EditPlan you wrote as `plan`. Only the bars and layers in the patch scope change; locked layers are never changed (listed in `refused`). Returns the patch, scope, changed_bars, changed_layers, a plain summary, and with `render` the checks. The edit is committed to the project's undo history. For a sound folder use praxis_ask instead, which also records a take.",
        input_schema = input_schema::<EditArgs>(),
        annotations(read_only_hint = false, destructive_hint = false, open_world_hint = true)
    )]
    async fn edit(
        &self,
        args: JsonObject,
        meta: RequestMetaObject,
        peer: Peer<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let req = match parse::<EditArgs>(args).and_then(|a| a.request()) {
            Ok(r) => r,
            Err(e) => return Ok(tool_result("edit", Err(e))),
        };
        Ok(run("edit", &meta, &peer, move || ops::edit(&req)).await)
    }

    #[tool(
        name = "praxis_variations",
        description = "Make N alternates (default 3) of a selection: `bars` and `layers` of a `project`, `score` or `sound`. Each is rendered to variations/<i>.wav next to the source and checked; nothing is committed. For a sound, keep one with praxis_keep_variation.",
        input_schema = input_schema::<VariationsArgs>(),
        annotations(read_only_hint = false, destructive_hint = false, open_world_hint = false)
    )]
    async fn variations(
        &self,
        args: JsonObject,
        meta: RequestMetaObject,
        peer: Peer<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let req = match parse::<VariationsArgs>(args).and_then(|a| a.request()) {
            Ok(r) => r,
            Err(e) => return Ok(tool_result("variations", Err(e))),
        };
        Ok(run("variations", &meta, &peer, move || ops::variations(&req)).await)
    }

    #[tool(
        name = "praxis_ask",
        description = "Make a take in a sound's shared history (the same thread the person sees in the app). On a new sound `text` is the brief (take 1); afterwards it is what to change, as a scoped edit of the current take. `scope` {layer, bars} holds the change to exactly those bars of that layer. With an API key (or `provider: \"claude\"`) the request is carried out by the sound assistant, which reads the sound, changes it with Praxis's own tools, renders and measures it, and remembers the sound's conversation; `agent.steps` lists what it did and `agent.reply` is its answer (a turn that makes no take, such as a clarifying question, fails with that reply and the steps under `error.issues[0].agent`). `provider: \"offline\"` uses the rule planner. Every take is rendered (takes/<n>/mix.wav), checked, and recorded with `changed` (bars per layer), `untouched` and `identical_bars` (bars measured bit-identical to the take before). The master gain is frozen after take 1, so untouched bars stay sample-identical.",
        input_schema = input_schema::<AskArgs>(),
        annotations(read_only_hint = false, destructive_hint = false, open_world_hint = true)
    )]
    async fn ask(
        &self,
        args: JsonObject,
        meta: RequestMetaObject,
        peer: Peer<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let who = client_who(&peer);
        let parsed = parse::<AskArgs>(args).and_then(|a| a.request(&who).map(|r| (r, a.options())));
        let (req, opts) = match parsed {
            Ok(r) => r,
            Err(e) => return Ok(tool_result("ask", Err(e))),
        };
        Ok(run("ask", &meta, &peer, move || ops::ask_with(&req, &opts)).await)
    }

    #[tool(
        name = "praxis_takes",
        description = "A sound's takes (who asked, what, the reply, what changed, the render path, the checks), the current take, the live score, the undo state and recent view requests. `revision` grows with every change, so compare it to see takes made by the person or other agents.",
        input_schema = input_schema::<TakesArgs>(),
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn takes(&self, args: JsonObject) -> Result<CallToolResult, ErrorData> {
        let result = parse::<TakesArgs>(args).and_then(|a| ops::takes(&a.request()));
        Ok(tool_result("takes", result))
    }

    #[tool(
        name = "praxis_restore",
        description = "Go back to take `n`: an undoable patch makes the score equal to that take again, and it becomes the current take (no new take is made).",
        input_schema = input_schema::<RestoreArgs>(),
        annotations(read_only_hint = false, destructive_hint = false, open_world_hint = false)
    )]
    async fn restore(
        &self,
        args: JsonObject,
        meta: RequestMetaObject,
        peer: Peer<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let req = match parse::<RestoreArgs>(args) {
            Ok(a) => a.request(),
            Err(e) => return Ok(tool_result("restore", Err(e))),
        };
        Ok(run("restore", &meta, &peer, move || ops::restore(&req)).await)
    }

    #[tool(
        name = "praxis_keep_variation",
        description = "Commit variation `index` from the sound's last praxis_variations run as a new take (rendered, checked and recorded like praxis_ask). Fails with patch_rejected when the sound changed since the variations were made.",
        input_schema = input_schema::<KeepVariationArgs>(),
        annotations(read_only_hint = false, destructive_hint = false, open_world_hint = false)
    )]
    async fn keep_variation(
        &self,
        args: JsonObject,
        meta: RequestMetaObject,
        peer: Peer<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let who = client_who(&peer);
        let req = match parse::<KeepVariationArgs>(args) {
            Ok(a) => a.request(&who),
            Err(e) => return Ok(tool_result("keep-variation", Err(e))),
        };
        Ok(run("keep-variation", &meta, &peer, move || {
            ops::keep_variation(&req)
        })
        .await)
    }

    #[tool(
        name = "praxis_open_view",
        description = "Ask the desktop app to show a panel view of the sound (overview, rhythm, notes, mix, export) to point the person at what you changed; the app shows \"<View> opened by <your name>\".",
        input_schema = input_schema::<OpenViewArgs>(),
        annotations(read_only_hint = false, destructive_hint = false, open_world_hint = false)
    )]
    async fn open_view(
        &self,
        args: JsonObject,
        meta: RequestMetaObject,
        peer: Peer<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let who = client_who(&peer);
        let req = match parse::<OpenViewArgs>(args).and_then(|a| a.request(&who)) {
            Ok(r) => r,
            Err(e) => return Ok(tool_result("open-view", Err(e))),
        };
        Ok(run("open-view", &meta, &peer, move || ops::open_view(&req)).await)
    }

    #[tool(
        name = "praxis_validate",
        description = "Validate a .score.json file or the score of a .praxis project: `valid`, `errors[]` and `warnings[]`, each issue with a code, a JSON pointer `path` and a message. Invalid scores fail with invalid_score and the errors under `error.issues`.",
        input_schema = input_schema::<ValidateArgs>(),
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn validate(&self, args: JsonObject) -> Result<CallToolResult, ErrorData> {
        let result = parse::<ValidateArgs>(args).and_then(|a| ops::validate(&a.request()));
        Ok(tool_result("validate", result))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for PraxisServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new("praxis-mcp", env!("CARGO_PKG_VERSION"))
                    .with_title("Praxis Sound"),
            )
            .with_instructions(guide::INSTRUCTIONS)
    }
}
