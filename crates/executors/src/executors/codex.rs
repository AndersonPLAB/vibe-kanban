pub mod client;
pub mod jsonrpc;
pub mod normalize_logs;
pub mod review;
pub mod slash_commands;
use std::{
    collections::HashMap,
    env,
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
};

/// Returns the Codex home directory.
///
/// Checks the `CODEX_HOME` environment variable first, then falls back to `~/.codex`.
/// This allows users to configure a custom location for Codex configuration and state.
pub fn codex_home() -> Option<PathBuf> {
    if let Ok(codex_home) = env::var("CODEX_HOME")
        && !codex_home.trim().is_empty()
    {
        return Some(PathBuf::from(codex_home));
    }
    dirs::home_dir().map(|home| home.join(".codex"))
}

pub(crate) fn resolve_model(model: Option<&str>) -> (Option<&str>, bool) {
    match model.and_then(|m| m.strip_suffix("-fast")) {
        Some(base) => (Some(base), true),
        None => (model, false),
    }
}

pub(crate) fn fork_params_from(thread_id: String, params: ThreadStartParams) -> ThreadForkParams {
    ThreadForkParams {
        thread_id,
        model: params.model,
        model_provider: params.model_provider,
        cwd: params.cwd,
        approval_policy: params.approval_policy,
        sandbox: params.sandbox,
        config: params.config,
        base_instructions: params.base_instructions,
        developer_instructions: params.developer_instructions,
        service_tier: params.service_tier,
        ..Default::default()
    }
}

use async_trait::async_trait;
use codex_app_server_protocol::{
    AskForApproval as V2AskForApproval, Model, ReviewTarget, SandboxMode as V2SandboxMode,
    ThreadForkParams, ThreadStartParams, UserInput,
};
use codex_protocol::{config_types::ServiceTier, openai_models::SPEED_TIER_FAST};
use derivative::Derivative;
use futures::StreamExt;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use strum_macros::{AsRefStr, EnumString};
use tokio::process::Command;
use ts_rs::TS;
use workspace_utils::{command_ext::GroupSpawnNoWindowExt, msg_store::MsgStore};

use self::{
    client::{AppServerClient, LogWriter},
    jsonrpc::{ExitSignalSender, JsonRpcPeer},
    normalize_logs::{Error, normalize_logs},
};
use crate::{
    approvals::ExecutorApprovalService,
    command::{CmdOverrides, CommandBuildError, CommandBuilder, CommandParts, apply_overrides},
    env::{ExecutionEnv, RepoContext},
    executor_discovery::ExecutorDiscoveredOptions,
    executors::{
        AppendPrompt, AvailabilityInfo, BaseCodingAgent, ExecutorError, ExecutorExitResult,
        SlashCommandDescription, SpawnedChild, StandardCodingAgentExecutor,
    },
    logs::utils::patch,
    model_selector::{ModelInfo, ModelSelectorConfig, PermissionPolicy, ReasoningOption},
    profile::ExecutorConfig,
    stdout_dup::create_stdout_pipe_writer,
};

/// Sandbox policy modes for Codex
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS, JsonSchema, AsRefStr)]
#[serde(rename_all = "kebab-case")]
#[strum(serialize_all = "kebab-case")]
pub enum SandboxMode {
    Auto,
    ReadOnly,
    WorkspaceWrite,
    DangerFullAccess,
}

/// Determines when the user is consulted to approve Codex actions.
///
/// - `UnlessTrusted`: Read-only commands are auto-approved. Everything else will
///   ask the user to approve.
/// - `OnFailure`: All commands run in a restricted sandbox initially. If a
///   command fails, the user is asked to approve execution without the sandbox.
/// - `OnRequest`: The model decides when to ask the user for approval.
/// - `Never`: Commands never ask for approval. Commands that fail in the
///   restricted sandbox are not retried.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS, JsonSchema, AsRefStr)]
#[serde(rename_all = "kebab-case")]
#[strum(serialize_all = "kebab-case")]
pub enum AskForApproval {
    UnlessTrusted,
    OnFailure,
    OnRequest,
    Never,
}

/// Reasoning effort for the underlying model
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS, JsonSchema, AsRefStr, EnumString)]
#[serde(rename_all = "kebab-case")]
#[strum(serialize_all = "kebab-case")]
pub enum ReasoningEffort {
    Low,
    Medium,
    High,
    Xhigh,
}

/// Model reasoning summary style
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS, JsonSchema, AsRefStr)]
#[serde(rename_all = "kebab-case")]
#[strum(serialize_all = "kebab-case")]
pub enum ReasoningSummary {
    Auto,
    Concise,
    Detailed,
    None,
}

/// Format for model reasoning summaries
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS, JsonSchema, AsRefStr)]
#[serde(rename_all = "kebab-case")]
#[strum(serialize_all = "kebab-case")]
pub enum ReasoningSummaryFormat {
    None,
    Experimental,
}

enum CodexSessionAction {
    Chat { prompt: String },
    Review { target: ReviewTarget },
}

/// How long we wait for `codex app-server` to answer `model/list` before
/// falling back to the static catalog.
const MODEL_DISCOVERY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// Static catalog used until `model/list` answers, and as fallback when the
/// app-server cannot be reached. Mirrors the catalog shipped with the pinned
/// CLI version.
fn default_discovered_options() -> ExecutorDiscoveredOptions {
    // zona de envelhecimento — revisar no merge mensal
    let static_models = [
        ("gpt-5.4", "GPT-5.4", true),
        ("gpt-5.4-mini", "GPT-5.4 Mini", false),
        ("gpt-5.3-codex", "GPT-5.3 Codex", false),
        ("gpt-5.2", "GPT-5.2", false),
    ];

    let reasoning_options = ReasoningOption::from_names_with_default(
        [
            ReasoningEffort::Low,
            ReasoningEffort::Medium,
            ReasoningEffort::High,
            ReasoningEffort::Xhigh,
        ]
        .map(|e| e.as_ref().to_string()),
        ReasoningEffort::Medium.as_ref(),
    );

    let models = static_models
        .into_iter()
        .flat_map(|(id, name, supports_fast)| {
            let base = ModelInfo {
                id: id.to_string(),
                name: name.to_string(),
                provider_id: None,
                reasoning_options: reasoning_options.clone(),
            };
            let fast = supports_fast.then(|| ModelInfo {
                id: format!("{id}-fast"),
                name: format!("{name} Fast"),
                provider_id: None,
                reasoning_options: reasoning_options.clone(),
            });
            std::iter::once(base).chain(fast)
        })
        .collect();

    ExecutorDiscoveredOptions {
        model_selector: ModelSelectorConfig {
            models,
            default_model: Some("gpt-5.4".to_string()),
            permissions: vec![
                PermissionPolicy::Auto,
                PermissionPolicy::Supervised,
                PermissionPolicy::Plan,
            ],
            ..Default::default()
        },
        slash_commands: vec![
            SlashCommandDescription {
                name: "compact".to_string(),
                description: Some(
                    "summarize conversation to prevent hitting the context limit".to_string(),
                ),
            },
            SlashCommandDescription {
                name: "init".to_string(),
                description: Some(
                    "create an AGENTS.md file with instructions for Codex".to_string(),
                ),
            },
            SlashCommandDescription {
                name: "status".to_string(),
                description: Some(
                    "show current session configuration and token usage".to_string(),
                ),
            },
            SlashCommandDescription {
                name: "mcp".to_string(),
                description: Some("list configured MCP tools".to_string()),
            },
            SlashCommandDescription {
                name: "model".to_string(),
                description: Some("view or switch the active model".to_string()),
            },
            SlashCommandDescription {
                name: "fast".to_string(),
                description: Some(
                    "toggle fast mode for highest speed inference (2× plan usage). Use `/fast on` or `/fast off` to set explicitly".to_string(),
                ),
            },
        ],
        ..Default::default()
    }
}

/// Map a `model/list` catalog to selector entries. Models advertising the
/// `fast` speed tier get an extra `<id>-fast` entry, which [`resolve_model`]
/// turns back into the base model plus `ServiceTier::Fast`.
fn models_from_catalog(catalog: &[Model]) -> Vec<ModelInfo> {
    catalog
        .iter()
        .flat_map(|model| {
            let reasoning_options = ReasoningOption::from_names_with_default(
                model
                    .supported_reasoning_efforts
                    .iter()
                    .map(|option| option.reasoning_effort.to_string()),
                &model.default_reasoning_effort.to_string(),
            );
            let base = ModelInfo {
                id: model.id.clone(),
                name: model.display_name.clone(),
                provider_id: None,
                reasoning_options: reasoning_options.clone(),
            };
            let fast = model
                .additional_speed_tiers
                .iter()
                .any(|tier| tier == SPEED_TIER_FAST)
                .then(|| ModelInfo {
                    id: format!("{}-fast", model.id),
                    name: format!("{} Fast", model.display_name),
                    provider_id: None,
                    reasoning_options,
                });
            std::iter::once(base).chain(fast)
        })
        .collect()
}

#[derive(Derivative, Clone, Serialize, Deserialize, TS, JsonSchema)]
#[derivative(Debug, PartialEq)]
pub struct Codex {
    #[serde(default)]
    pub append_prompt: AppendPrompt,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox: Option<SandboxMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ask_for_approval: Option<AskForApproval>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oss: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_reasoning_effort: Option<ReasoningEffort>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_reasoning_summary: Option<ReasoningSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_reasoning_summary_format: Option<ReasoningSummaryFormat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_instructions: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_apply_patch_tool: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compact_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub developer_instructions: Option<String>,
    #[serde(default)]
    pub plan: bool,
    #[serde(flatten)]
    pub cmd: CmdOverrides,

    #[serde(skip)]
    #[ts(skip)]
    #[derivative(Debug = "ignore", PartialEq = "ignore")]
    approvals: Option<Arc<dyn ExecutorApprovalService>>,
}

#[async_trait]
impl StandardCodingAgentExecutor for Codex {
    fn apply_overrides(&mut self, executor_config: &ExecutorConfig) {
        if let Some(model_id) = &executor_config.model_id {
            self.model = Some(model_id.clone());
        }
        if let Some(reasoning_id) = &executor_config.reasoning_id
            && let Ok(reasoning_effort) = ReasoningEffort::from_str(reasoning_id)
        {
            self.model_reasoning_effort = Some(reasoning_effort)
        }
        if let Some(permission_policy) = &executor_config.permission_policy {
            match permission_policy {
                crate::model_selector::PermissionPolicy::Auto => {
                    self.ask_for_approval = Some(AskForApproval::Never);
                    self.plan = false;
                }
                crate::model_selector::PermissionPolicy::Supervised => {
                    if matches!(self.ask_for_approval, None | Some(AskForApproval::Never)) {
                        self.ask_for_approval = Some(AskForApproval::UnlessTrusted);
                    }
                    self.plan = false;
                }
                crate::model_selector::PermissionPolicy::Plan => {
                    self.plan = true;
                }
            }
        }
    }

    fn use_approvals(&mut self, approvals: Arc<dyn ExecutorApprovalService>) {
        self.approvals = Some(approvals);
    }

    async fn spawn(
        &self,
        current_dir: &Path,
        prompt: &str,
        env: &ExecutionEnv,
    ) -> Result<SpawnedChild, ExecutorError> {
        self.spawn_slash_command(current_dir, prompt, None, env)
            .await
    }

    async fn spawn_follow_up(
        &self,
        current_dir: &Path,
        prompt: &str,
        session_id: &str,
        _reset_to_message_id: Option<&str>,
        env: &ExecutionEnv,
    ) -> Result<SpawnedChild, ExecutorError> {
        self.spawn_slash_command(current_dir, prompt, Some(session_id), env)
            .await
    }

    fn normalize_logs(
        &self,
        msg_store: Arc<MsgStore>,
        worktree_path: &Path,
    ) -> Vec<tokio::task::JoinHandle<()>> {
        normalize_logs(msg_store, worktree_path)
    }

    fn default_mcp_config_path(&self) -> Option<PathBuf> {
        codex_home().map(|home| home.join("config.toml"))
    }

    fn get_availability_info(&self) -> AvailabilityInfo {
        if let Some(timestamp) = codex_home()
            .and_then(|home| std::fs::metadata(home.join("auth.json")).ok())
            .and_then(|m| m.modified().ok())
            .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
        {
            return AvailabilityInfo::LoginDetected {
                last_auth_timestamp: timestamp,
            };
        }

        let mcp_config_found = self
            .default_mcp_config_path()
            .map(|p| p.exists())
            .unwrap_or(false);

        let installation_indicator_found = codex_home()
            .map(|home| home.join("version.json").exists())
            .unwrap_or(false);

        if mcp_config_found || installation_indicator_found {
            AvailabilityInfo::InstallationFound
        } else {
            AvailabilityInfo::NotFound
        }
    }

    fn get_preset_options(&self) -> ExecutorConfig {
        use crate::model_selector::*;
        let permission_policy = if self.plan {
            PermissionPolicy::Plan
        } else if matches!(self.ask_for_approval, None | Some(AskForApproval::Never)) {
            PermissionPolicy::Auto
        } else {
            PermissionPolicy::Supervised
        };

        ExecutorConfig {
            executor: BaseCodingAgent::Codex,
            variant: None,
            model_id: self.model.clone(),
            agent_id: None,
            reasoning_id: self
                .model_reasoning_effort
                .as_ref()
                .map(|e| e.as_ref().to_string()),
            permission_policy: Some(permission_policy),
        }
    }

    async fn discover_options(
        &self,
        workdir: Option<&std::path::Path>,
        repo_path: Option<&std::path::Path>,
    ) -> Result<futures::stream::BoxStream<'static, json_patch::Patch>, ExecutorError> {
        use crate::{
            executor_discovery::ExecutorConfigCacheKey, executors::utils::executor_options_cache,
        };

        // The Codex catalog is account-scoped, not per-directory, so a single
        // global cache entry is enough.
        let cache_key =
            ExecutorConfigCacheKey::new(None, self.compute_cmd_key(), BaseCodingAgent::Codex);
        if let Some(cached) = executor_options_cache().get(&cache_key) {
            let options = cached.as_ref().clone().with_loading(false);
            return Ok(Box::pin(futures::stream::once(async move {
                patch::executor_discovered_options(options)
            })));
        }

        let mut initial_options = default_discovered_options();
        initial_options.loading_models = true;
        let initial_patch = patch::executor_discovered_options(initial_options);

        let this = self.clone();
        let discovery_path = workdir
            .or(repo_path)
            .unwrap_or(Path::new("."))
            .to_path_buf();

        let discovery_stream = async_stream::stream! {
            let mut final_options = default_discovered_options();

            match this.list_models(&discovery_path).await {
                // An empty catalog would leave the picker with nothing to pick.
                Ok(catalog) if !catalog.is_empty() => {
                    final_options.model_selector.models = models_from_catalog(&catalog);
                    final_options.model_selector.default_model = catalog
                        .iter()
                        .find(|model| model.is_default)
                        .map(|model| model.id.clone());
                }
                Ok(_) => {
                    tracing::warn!("Codex returned an empty model list, keeping static fallback");
                }
                Err(e) => {
                    tracing::warn!("Failed to list Codex models, keeping static fallback: {e}");
                }
            }

            yield patch::update_models(final_options.model_selector.models.clone());
            yield patch::update_default_model(final_options.model_selector.default_model.clone());
            yield patch::models_loaded();

            executor_options_cache().put(cache_key, final_options);
        };

        Ok(Box::pin(
            futures::stream::once(async move { initial_patch }).chain(discovery_stream),
        ))
    }

    async fn spawn_review(
        &self,
        current_dir: &Path,
        prompt: &str,
        session_id: Option<&str>,
        env: &ExecutionEnv,
    ) -> Result<SpawnedChild, ExecutorError> {
        let command_parts = self.build_command_builder()?.build_initial()?;
        let review_target = ReviewTarget::Custom {
            instructions: prompt.to_string(),
        };
        let action = CodexSessionAction::Review {
            target: review_target,
        };
        self.spawn_inner(current_dir, command_parts, action, session_id, env)
            .await
    }
}

impl Codex {
    pub fn base_command() -> &'static str {
        "npx -y @openai/codex@0.124.0"
    }

    fn build_command_builder(&self) -> Result<CommandBuilder, CommandBuildError> {
        let mut builder = CommandBuilder::new(Self::base_command());
        builder = builder.extend_params(["app-server"]);
        if self.oss.unwrap_or(false) {
            builder = builder.extend_params(["--oss"]);
        }

        apply_overrides(builder, &self.cmd)
    }

    fn compute_cmd_key(&self) -> String {
        serde_json::to_string(&self.cmd).unwrap_or_default()
    }

    /// Ask a throwaway `codex app-server` for the model catalog. `model/list`
    /// is a metadata request: no thread is started, so no tokens are spent.
    async fn list_models(&self, current_dir: &Path) -> Result<Vec<Model>, ExecutorError> {
        let command_parts = self.build_command_builder()?.build_initial()?;
        let (program_path, args) = command_parts.into_resolved().await?;

        let mut process = Command::new(program_path);
        process
            .kill_on_drop(true)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .current_dir(current_dir)
            .env("NPM_CONFIG_LOGLEVEL", "error")
            .env("NODE_NO_WARNINGS", "1")
            .env("NO_COLOR", "1")
            .env("RUST_LOG", "error")
            .args(&args);

        ExecutionEnv::new(RepoContext::default(), false, String::new())
            .with_profile(&self.cmd)
            .apply_to_command(&mut process);

        let mut child = process.group_spawn_no_window()?;
        let child_stdout = child.inner().stdout.take().ok_or_else(|| {
            ExecutorError::Io(std::io::Error::other("Codex app server missing stdout"))
        })?;
        let child_stdin = child.inner().stdin.take().ok_or_else(|| {
            ExecutorError::Io(std::io::Error::other("Codex app server missing stdin"))
        })?;

        let cancel = tokio_util::sync::CancellationToken::new();
        let (exit_signal_tx, _exit_signal_rx) = tokio::sync::oneshot::channel();
        let client = AppServerClient::new(
            LogWriter::new(tokio::io::sink()),
            None,
            false,
            false,
            RepoContext::default(),
            false,
            String::new(),
            cancel.clone(),
        );
        let rpc_peer = JsonRpcPeer::spawn(
            child_stdin,
            child_stdout,
            client.clone(),
            ExitSignalSender::new(exit_signal_tx),
            cancel.clone(),
        );
        client.connect(rpc_peer);

        let result = tokio::time::timeout(MODEL_DISCOVERY_TIMEOUT, async {
            client.initialize().await?;
            Ok::<_, ExecutorError>(client.model_list().await?.data)
        })
        .await;

        cancel.cancel();
        let _ = child.kill().await;

        result.unwrap_or_else(|_| {
            Err(ExecutorError::Io(std::io::Error::other(
                "Timed out listing Codex models",
            )))
        })
    }

    fn build_thread_start_params(&self, cwd: &Path) -> ThreadStartParams {
        let sandbox = match self.sandbox.as_ref() {
            None | Some(SandboxMode::Auto) => Some(V2SandboxMode::WorkspaceWrite), // match the Auto preset in codex
            Some(SandboxMode::ReadOnly) => Some(V2SandboxMode::ReadOnly),
            Some(SandboxMode::WorkspaceWrite) => Some(V2SandboxMode::WorkspaceWrite),
            Some(SandboxMode::DangerFullAccess) => Some(V2SandboxMode::DangerFullAccess),
        };

        let approval_policy = match self.ask_for_approval.as_ref() {
            None if matches!(self.sandbox.as_ref(), None | Some(SandboxMode::Auto)) => {
                // match the Auto preset in codex
                Some(V2AskForApproval::OnRequest)
            }
            None => None,
            Some(AskForApproval::UnlessTrusted) => Some(V2AskForApproval::UnlessTrusted),
            Some(AskForApproval::OnFailure) => Some(V2AskForApproval::OnFailure),
            Some(AskForApproval::OnRequest) => Some(V2AskForApproval::OnRequest),
            Some(AskForApproval::Never) => Some(V2AskForApproval::Never),
        };

        let mut config = self.build_config_overrides();
        // V1 top-level params that moved into config overrides in v2
        if let Some(profile) = &self.profile {
            config
                .get_or_insert_with(HashMap::new)
                .insert("profile".to_string(), Value::String(profile.clone()));
        }
        if let Some(include) = self.include_apply_patch_tool {
            config
                .get_or_insert_with(HashMap::new)
                .insert("include_apply_patch_tool".to_string(), Value::Bool(include));
        }
        if let Some(compact) = &self.compact_prompt {
            config
                .get_or_insert_with(HashMap::new)
                .insert("compact_prompt".to_string(), Value::String(compact.clone()));
        }
        if !matches!(approval_policy, None | Some(V2AskForApproval::Never)) {
            let map = config.get_or_insert_with(HashMap::new);
            map.insert(
                "features.default_mode_request_user_input".to_string(),
                Value::Bool(true),
            );
            map.insert(
                "suppress_unstable_features_warning".to_string(),
                Value::Bool(true),
            );
        }

        let (model, is_fast) = resolve_model(self.model.as_deref());
        let service_tier = if is_fast {
            Some(Some(ServiceTier::Fast))
        } else {
            None
        };

        ThreadStartParams {
            model: model.map(|m| m.to_string()),
            cwd: Some(cwd.to_string_lossy().to_string()),
            approval_policy,
            sandbox,
            config,
            base_instructions: self.base_instructions.clone(),
            model_provider: self.model_provider.clone(),
            developer_instructions: self.developer_instructions.clone(),
            service_tier,
            ..Default::default()
        }
    }

    fn build_config_overrides(&self) -> Option<HashMap<String, Value>> {
        let mut overrides = HashMap::new();

        if let Some(effort) = &self.model_reasoning_effort {
            overrides.insert(
                "model_reasoning_effort".to_string(),
                Value::String(effort.as_ref().to_string()),
            );
        }

        if let Some(summary) = &self.model_reasoning_summary {
            overrides.insert(
                "model_reasoning_summary".to_string(),
                Value::String(summary.as_ref().to_string()),
            );
        }

        if let Some(format) = &self.model_reasoning_summary_format
            && format != &ReasoningSummaryFormat::None
        {
            overrides.insert(
                "model_reasoning_summary_format".to_string(),
                Value::String(format.as_ref().to_string()),
            );
        }

        if overrides.is_empty() {
            None
        } else {
            Some(overrides)
        }
    }

    async fn spawn_inner(
        &self,
        current_dir: &Path,
        command_parts: CommandParts,
        action: CodexSessionAction,
        resume_session: Option<&str>,
        env: &ExecutionEnv,
    ) -> Result<SpawnedChild, ExecutorError> {
        let params = self.build_thread_start_params(current_dir);
        let resume_session = resume_session.map(|s| s.to_string());

        self.spawn_app_server(
            current_dir,
            command_parts,
            env,
            move |client, _| async move {
                match action {
                    CodexSessionAction::Chat { prompt } => {
                        Self::launch_codex_agent(params, resume_session, prompt, client).await
                    }
                    CodexSessionAction::Review { target } => {
                        review::launch_codex_review(params, resume_session, target, client).await
                    }
                }
            },
        )
        .await
    }

    async fn launch_codex_agent(
        thread_start_params: ThreadStartParams,
        resume_session: Option<String>,
        combined_prompt: String,
        client: Arc<AppServerClient>,
    ) -> Result<(), ExecutorError> {
        let account = client.get_account().await?;
        if account.requires_openai_auth && account.account.is_none() {
            return Err(ExecutorError::AuthRequired(
                "Codex authentication required".to_string(),
            ));
        }

        let (thread_id, resolved_model) = match resume_session {
            None => {
                let response = client.thread_start(thread_start_params).await?;
                (response.thread.id, response.model)
            }
            Some(session_id) => {
                let response = client
                    .thread_fork(fork_params_from(session_id, thread_start_params))
                    .await?;
                tracing::debug!("forked thread, new thread_id={}", response.thread.id);
                (response.thread.id, response.model)
            }
        };

        client.set_resolved_model(resolved_model);
        client.register_session(&thread_id).await?;
        let collaboration_mode = client.initial_collaboration_mode()?;
        client
            .turn_start_with_mode(
                thread_id,
                vec![UserInput::Text {
                    text: combined_prompt,
                    text_elements: vec![],
                }],
                Some(collaboration_mode),
            )
            .await?;

        Ok(())
    }

    /// Common boilerplate for spawning a Codex app server process
    /// Handles process spawning, stdout/stderr piping, exit signal handling, client initialization, and error logging.
    /// Delegates the actual Codex session logic to the provided `task` closure.
    async fn spawn_app_server<F, Fut>(
        &self,
        current_dir: &Path,
        command_parts: CommandParts,
        env: &ExecutionEnv,
        task: F,
    ) -> Result<SpawnedChild, ExecutorError>
    where
        F: FnOnce(Arc<AppServerClient>, ExitSignalSender) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<(), ExecutorError>> + Send + 'static,
    {
        let (program_path, args) = command_parts.into_resolved().await?;

        let mut process = Command::new(program_path);
        process
            .kill_on_drop(true)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .current_dir(current_dir)
            .env("NPM_CONFIG_LOGLEVEL", "error")
            .env("NODE_NO_WARNINGS", "1")
            .env("NO_COLOR", "1")
            .env("RUST_LOG", "error")
            .args(&args);

        env.clone()
            .with_profile(&self.cmd)
            .apply_to_command(&mut process);

        let mut child = process.group_spawn_no_window()?;

        let child_stdout = child.inner().stdout.take().ok_or_else(|| {
            ExecutorError::Io(std::io::Error::other("Codex app server missing stdout"))
        })?;
        let child_stdin = child.inner().stdin.take().ok_or_else(|| {
            ExecutorError::Io(std::io::Error::other("Codex app server missing stdin"))
        })?;

        let new_stdout = create_stdout_pipe_writer(&mut child)?;
        let (exit_signal_tx, exit_signal_rx) = tokio::sync::oneshot::channel();
        let cancel = tokio_util::sync::CancellationToken::new();

        let auto_approve = matches!(
            (&self.sandbox, &self.ask_for_approval),
            (Some(SandboxMode::DangerFullAccess), None)
        );
        let plan_mode = self.plan;
        let approvals = self.approvals.clone();
        let repo_context = env.repo_context.clone();
        let commit_reminder = env.commit_reminder;
        let commit_reminder_prompt = env.commit_reminder_prompt.clone();
        let cancel_for_task = cancel.clone();

        tokio::spawn(async move {
            let exit_signal_tx = ExitSignalSender::new(exit_signal_tx);
            let log_writer = LogWriter::new(new_stdout);

            // Initialize the AppServerClient
            let client = AppServerClient::new(
                log_writer.clone(),
                approvals,
                auto_approve,
                plan_mode,
                repo_context,
                commit_reminder,
                commit_reminder_prompt,
                cancel_for_task.clone(),
            );
            let rpc_peer = JsonRpcPeer::spawn(
                child_stdin,
                child_stdout,
                client.clone(),
                exit_signal_tx.clone(),
                cancel_for_task,
            );
            client.connect(rpc_peer);

            let result = async {
                client.initialize().await?;
                task(client, exit_signal_tx.clone()).await
            }
            .await;

            if let Err(err) = result {
                match &err {
                    ExecutorError::Io(io_err)
                        if io_err.kind() == std::io::ErrorKind::BrokenPipe =>
                    {
                        // Broken pipe likely means the parent process exited, so we can ignore it
                        return;
                    }
                    ExecutorError::AuthRequired(message) => {
                        log_writer
                            .log_raw(&Error::auth_required(message.clone()).raw())
                            .await
                            .ok();
                        exit_signal_tx
                            .send_exit_signal(ExecutorExitResult::Failure)
                            .await;
                        return;
                    }
                    _ => {
                        tracing::error!("Codex spawn error: {}", err);
                        log_writer
                            .log_raw(&Error::launch_error(err.to_string()).raw())
                            .await
                            .ok();
                    }
                }
                exit_signal_tx
                    .send_exit_signal(ExecutorExitResult::Failure)
                    .await;
            }
        });

        Ok(SpawnedChild {
            child,
            exit_signal: Some(exit_signal_rx),
            cancel: Some(cancel),
        })
    }
}

#[cfg(test)]
mod tests {
    use codex_app_server_protocol::Model;

    use super::{models_from_catalog, resolve_model};

    /// Trimmed capture of a real `model/list` response from
    /// `npx -y @openai/codex@0.124.0 app-server`.
    const MODEL_LIST_SAMPLE: &str = r#"[
        {
            "id": "gpt-5.4", "model": "gpt-5.4", "upgrade": null, "upgradeInfo": null,
            "availabilityNux": null, "displayName": "gpt-5.4", "description": "Latest.",
            "hidden": false,
            "supportedReasoningEfforts": [
                {"reasoningEffort": "low", "description": "l"},
                {"reasoningEffort": "medium", "description": "m"},
                {"reasoningEffort": "high", "description": "h"},
                {"reasoningEffort": "xhigh", "description": "x"}
            ],
            "defaultReasoningEffort": "medium",
            "inputModalities": ["text"], "supportsPersonality": true,
            "additionalSpeedTiers": ["fast"], "isDefault": true
        },
        {
            "id": "gpt-5.4-mini", "model": "gpt-5.4-mini", "upgrade": null, "upgradeInfo": null,
            "availabilityNux": null, "displayName": "GPT-5.4-Mini", "description": "Smaller.",
            "hidden": false,
            "supportedReasoningEfforts": [
                {"reasoningEffort": "low", "description": "l"},
                {"reasoningEffort": "medium", "description": "m"}
            ],
            "defaultReasoningEffort": "medium",
            "inputModalities": ["text"], "supportsPersonality": true,
            "additionalSpeedTiers": [], "isDefault": false
        }
    ]"#;

    #[test]
    fn maps_model_list_catalog_to_selector_models() {
        let catalog: Vec<Model> = serde_json::from_str(MODEL_LIST_SAMPLE).unwrap();
        let models = models_from_catalog(&catalog);

        let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
        // The `fast` speed tier becomes a separate selectable id.
        assert_eq!(ids, ["gpt-5.4", "gpt-5.4-fast", "gpt-5.4-mini"]);

        // Reasoning options come from the model itself, not from a fixed list.
        let mini = models.last().unwrap();
        let mini_efforts: Vec<&str> = mini
            .reasoning_options
            .iter()
            .map(|o| o.id.as_str())
            .collect();
        assert_eq!(mini_efforts, ["low", "medium"]);
        assert!(
            mini.reasoning_options
                .iter()
                .find(|o| o.id == "medium")
                .unwrap()
                .is_default
        );
    }

    /// Live round-trip against the pinned CLI: spawns `codex app-server`,
    /// initializes and asks for `model/list`. Metadata only — no thread is
    /// started, so it costs no tokens. Ignored by default because it needs the
    /// CLI, network and a logged-in account.
    #[tokio::test]
    #[ignore = "spawns the real codex app-server"]
    async fn model_list_live_round_trip() {
        let codex: super::Codex = serde_json::from_value(serde_json::json!({})).unwrap();
        let catalog = codex.list_models(std::path::Path::new(".")).await.unwrap();
        let models = models_from_catalog(&catalog);
        println!("{:#?}", models);
        assert!(!models.is_empty());
        assert!(catalog.iter().any(|m| m.is_default));
    }

    #[test]
    fn resolve_model_detects_fast_suffix() {
        assert_eq!(resolve_model(Some("gpt-5.5-fast")), (Some("gpt-5.5"), true));
        assert_eq!(resolve_model(Some("gpt-5.4-fast")), (Some("gpt-5.4"), true));
    }

    #[test]
    fn resolve_model_leaves_non_fast_models_unchanged() {
        assert_eq!(resolve_model(Some("gpt-5.5")), (Some("gpt-5.5"), false));
        assert_eq!(
            resolve_model(Some("gpt-5.4-mini")),
            (Some("gpt-5.4-mini"), false)
        );
        assert_eq!(resolve_model(None), (None, false));
    }
}
