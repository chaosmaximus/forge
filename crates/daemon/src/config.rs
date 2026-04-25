// config.rs — ~/.forge/config.toml parser

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Default value helpers (for serde(default = "fn"))
// ---------------------------------------------------------------------------

fn default_3() -> u64 {
    3
}
fn default_5_usize() -> usize {
    5
}
fn default_3_usize() -> usize {
    3
}
fn default_10_usize() -> usize {
    10
}
fn default_15() -> u64 {
    15
}
fn default_30() -> u64 {
    30
}
fn default_50_usize() -> usize {
    50
}
fn default_60() -> u64 {
    60
}
fn default_200_usize() -> usize {
    200
}
fn default_300() -> u64 {
    300
}
fn default_600() -> u64 {
    600
}
fn default_900() -> u64 {
    900
}
fn default_1800() -> u64 {
    1800
}
fn default_14400() -> u64 {
    14_400
}
fn default_21600() -> u64 {
    21_600
}
fn default_kpi_retention_days() -> u32 {
    30
}
fn default_3000_usize() -> usize {
    3000
}
fn default_5000_usize() -> usize {
    5000
}
fn default_300_usize() -> usize {
    300
}
fn default_500_usize() -> usize {
    500
}
fn default_anti_pattern_threshold() -> f64 {
    0.85
}
fn default_completion_keywords() -> Vec<String> {
    vec![
        "complete".into(),
        "completed".into(),
        "done".into(),
        "finished".into(),
        "shipped".into(),
        "all tests pass".into(),
        "100%".into(),
        "no gaps".into(),
        "zero issues".into(),
        "pushed".into(),
    ]
}
fn default_true() -> bool {
    true
}
fn default_false() -> bool {
    false
}
fn default_300_u64() -> u64 {
    300
}
fn default_10_u64() -> u64 {
    10
}
fn default_3600_u64() -> u64 {
    3600
}
fn default_30_u32() -> u32 {
    30
}
fn default_1_f64() -> f64 {
    1.0
}
fn default_8420_u16() -> u16 {
    8420
}
fn default_8421_u16() -> u16 {
    8421
}
fn default_bind() -> String {
    "127.0.0.1".to_string()
}
fn default_grpc_bind() -> String {
    "127.0.0.1".to_string()
}
fn default_cors_origins() -> Vec<String> {
    vec![
        "http://localhost:*".to_string(),
        "https://localhost:*".to_string(),
        "http://127.0.0.1:*".to_string(),
        "https://127.0.0.1:*".to_string(),
    ]
}
fn default_service_name() -> String {
    "forge-daemon".to_string()
}
fn default_healing_cosine() -> f64 {
    0.65
}
fn default_healing_overlap_low() -> f64 {
    0.3
}
fn default_healing_overlap_high() -> f64 {
    0.7
}
fn default_healing_staleness_days() -> u64 {
    7
}
fn default_healing_staleness_min_quality() -> f64 {
    0.2
}
fn default_healing_quality_decay() -> f64 {
    0.1
}
fn default_healing_quality_boost() -> f64 {
    0.05
}
fn default_workspace_mode() -> String {
    "project".to_string()
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ForgeConfig {
    pub extraction: ExtractionConfig,
    pub embedding: EmbeddingConfig,
    pub a2a: A2aConfig,
    pub workers: WorkerConfig,
    pub context: ContextConfig,
    #[serde(default)]
    pub context_injection: ContextInjectionConfig,
    pub consolidation: ConsolidationConfig,
    #[serde(default)]
    pub recall: RecallConfig,
    #[serde(default)]
    pub reality: RealityConfig,
    #[serde(default)]
    pub meeting: MeetingConfig,
    #[serde(default)]
    pub agent: AgentConfig,
    #[serde(default)]
    pub http: HttpConfig,
    #[serde(default)]
    pub grpc: GrpcConfig,
    #[serde(default)]
    pub cors: CorsConfig,
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(default)]
    pub metrics: MetricsConfig,
    #[serde(default)]
    pub otlp: OtlpConfig,
    #[serde(default)]
    pub proactive: ProactiveConfig,
    #[serde(default)]
    pub healing: HealingConfig,
    #[serde(default)]
    pub tls: TlsConfig,
    #[serde(default)]
    pub ui: UiConfig,
    #[serde(default)]
    pub workspace: WorkspaceConfig,
    #[serde(default)]
    pub license: LicenseConfig,
    /// Path to the skills directory for the skills registry.
    /// Defaults to "product/claude-skills" relative to the project root.
    #[serde(default)]
    pub skills_directory: String,
}

/// HTTP transport configuration — opt-in, disabled by default.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HttpConfig {
    #[serde(default = "default_false")]
    pub enabled: bool,
    #[serde(default = "default_bind")]
    pub bind: String,
    #[serde(default = "default_8420_u16")]
    pub port: u16,
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bind: "127.0.0.1".to_string(),
            port: 8420,
        }
    }
}

/// TLS configuration — opt-in, disabled by default.
/// When enabled, the daemon generates a self-signed certificate for localhost
/// and serves HTTPS. Users can install the CA cert to trust Forge in their browser.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TlsConfig {
    #[serde(default = "default_false")]
    pub enabled: bool,
}

/// License tier configuration.
/// Controls which features are available based on the subscription plan.
/// Default: tier="free", key="" (no license key required for free tier).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LicenseConfig {
    /// Current tier: "free", "pro", "team", "enterprise"
    pub tier: String,
    /// License key (validated on startup, empty = free tier)
    pub key: String,
}

impl Default for LicenseConfig {
    fn default() -> Self {
        Self {
            tier: "free".to_string(),
            key: String::new(),
        }
    }
}

/// gRPC transport configuration — opt-in, disabled by default.
/// Uses JSON-over-gRPC: a single Execute RPC carrying JSON-serialized
/// Request/Response, giving HTTP/2 + mTLS + streaming without mirroring
/// all protocol variants in Protobuf.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GrpcConfig {
    #[serde(default = "default_false")]
    pub enabled: bool,
    #[serde(default = "default_grpc_bind")]
    pub bind: String,
    #[serde(default = "default_8421_u16")]
    pub port: u16,
}

impl Default for GrpcConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bind: "127.0.0.1".to_string(),
            port: 8421,
        }
    }
}

/// CORS configuration for HTTP transport.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CorsConfig {
    #[serde(default = "default_cors_origins")]
    pub allowed_origins: Vec<String>,
    #[serde(default = "default_3600_u64")]
    pub max_age_secs: u64,
}

impl Default for CorsConfig {
    fn default() -> Self {
        Self {
            allowed_origins: vec![
                "http://localhost:*".to_string(),
                "https://localhost:*".to_string(),
                "http://127.0.0.1:*".to_string(),
                "https://127.0.0.1:*".to_string(),
            ],
            max_age_secs: 3600,
        }
    }
}

/// Auth configuration for HTTP transport — JWT/OIDC based, disabled by default.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AuthConfig {
    #[serde(default = "default_false")]
    pub enabled: bool,
    #[serde(default)]
    pub issuer_url: String,
    #[serde(default)]
    pub audience: String,
    #[serde(default)]
    pub required_claims: Vec<String>,
    #[serde(default)]
    pub admin_emails: Vec<String>,
    /// Emails assigned the Viewer role (read-only access).
    /// Users not in admin_emails or viewer_emails default to Member.
    #[serde(default)]
    pub viewer_emails: Vec<String>,
    #[serde(default = "default_3600_u64")]
    pub jwks_cache_secs: u64,
    #[serde(default)]
    pub offline_jwks_path: Option<String>,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            issuer_url: String::new(),
            audience: String::new(),
            required_claims: Vec::new(),
            admin_emails: Vec::new(),
            viewer_emails: Vec::new(),
            jwks_cache_secs: 3600,
            offline_jwks_path: None,
        }
    }
}

/// Prometheus metrics configuration — enabled by default.
/// Override with FORGE_METRICS_ENABLED=false to disable the /metrics endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MetricsConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// OTLP trace export configuration — opt-in, disabled by default.
/// When enabled, spans are exported via gRPC to a collector (Jaeger, Datadog, LangSmith, etc.).
/// Override with FORGE_OTLP_ENABLED, FORGE_OTLP_ENDPOINT, FORGE_OTLP_SERVICE_NAME env vars.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OtlpConfig {
    #[serde(default = "default_false")]
    pub enabled: bool,
    /// gRPC endpoint for OTLP collector, e.g. "http://localhost:4317"
    #[serde(default)]
    pub endpoint: String,
    /// Service name reported in traces
    #[serde(default = "default_service_name")]
    pub service_name: String,
}

impl Default for OtlpConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint: String::new(),
            service_name: "forge-daemon".to_string(),
        }
    }
}

/// Worker interval configuration — all values in seconds.
/// Defaults match the previously hardcoded constants for zero behavioral change.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkerConfig {
    #[serde(default = "default_15")]
    pub extraction_debounce_secs: u64,
    #[serde(default = "default_1800")]
    pub consolidation_interval_secs: u64,
    #[serde(default = "default_60")]
    pub embedding_interval_secs: u64,
    #[serde(default = "default_30")]
    pub perception_interval_secs: u64,
    #[serde(default = "default_900")]
    pub disposition_interval_secs: u64,
    #[serde(default = "default_300")]
    pub indexer_interval_secs: u64,
    #[serde(default = "default_3")]
    pub diagnostics_debounce_secs: u64,
    /// How often the session reaper runs to clean up dead sessions (seconds).
    #[serde(default = "default_60")]
    pub session_reaper_interval_secs: u64,
    /// Sessions whose last heartbeat is older than this transition `active`
    /// → `ended` (seconds). Default 14400 (4h) — the previous default of 60s
    /// was too aggressive for real-world operator workflows where users
    /// step away mid-task. Set lower if you want strict reaping.
    ///
    /// Phase 2A-4d.3.1 #7 — paired with `heartbeat_idle_secs` for the
    /// `active → idle → ended` lifecycle.
    #[serde(default = "default_14400")]
    pub heartbeat_timeout_secs: u64,
    /// Sessions whose last heartbeat is older than this (but newer than
    /// `heartbeat_timeout_secs`) transition `active` → `idle` (seconds).
    /// Default 600 (10 min). Set to 0 to disable the idle phase
    /// (sessions go straight from active to ended).
    ///
    /// Phase 2A-4d.3.1 #7 — operators saw "many active sessions" because
    /// a session is `active` for the entire heartbeat window. The idle
    /// phase makes the dormant subset visible without ending them yet.
    #[serde(default = "default_600")]
    pub heartbeat_idle_secs: u64,
    /// Phase 2A-4d.2 T7: retention window for the `kpi_events` table (days).
    /// Rows older than this are deleted on every `kpi_reaper_interval_secs`
    /// tick. Default 30 days is sized so `/inspect` dashboards keep a month
    /// of history while `phase_completed` volume (~1,100 rows/project/day)
    /// stays bounded.
    #[serde(default = "default_kpi_retention_days")]
    pub kpi_events_retention_days: u32,
    /// Phase 2A-4d.2 T7: how often the `kpi_events` retention reaper fires
    /// (seconds). Default 6h = 21600s; slow cadence because deletions are
    /// batched and the cutoff only moves by one tick anyway.
    #[serde(default = "default_21600")]
    pub kpi_reaper_interval_secs: u64,
    /// Per-event-type retention override map. When a kpi_events row's
    /// event_type matches a key, that retention applies; otherwise the
    /// global `kpi_events_retention_days` is used.
    ///
    /// Default seeds `bench_run_completed` = 180 days for the leaderboard
    /// (Tier 3 D9). Operators can extend or override.
    #[serde(default = "default_kpi_events_retention_days_by_type")]
    pub kpi_events_retention_days_by_type: HashMap<String, u32>,
}

fn default_kpi_events_retention_days_by_type() -> HashMap<String, u32> {
    let mut m = HashMap::new();
    m.insert("bench_run_completed".to_string(), 180);
    m
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            extraction_debounce_secs: 15,
            consolidation_interval_secs: 1800,
            embedding_interval_secs: 60,
            perception_interval_secs: 30,
            disposition_interval_secs: 900,
            indexer_interval_secs: 300,
            diagnostics_debounce_secs: 3,
            session_reaper_interval_secs: 60,
            heartbeat_timeout_secs: 14_400, // 4h — see field doc
            heartbeat_idle_secs: 600,       // 10 min
            kpi_events_retention_days: 30,
            kpi_reaper_interval_secs: 21_600,
            kpi_events_retention_days_by_type: default_kpi_events_retention_days_by_type(),
        }
    }
}

/// Context assembly configuration — limits and budget for compile_context.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ContextConfig {
    #[serde(default = "default_3000_usize")]
    pub budget_chars: usize,
    #[serde(default = "default_10_usize")]
    pub decisions_limit: usize,
    #[serde(default = "default_5_usize")]
    pub lessons_limit: usize,
    #[serde(default = "default_5_usize")]
    pub entities_limit: usize,
    #[serde(default = "default_3_usize")]
    pub entities_min_mentions: usize,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            budget_chars: 3000,
            decisions_limit: 10,
            lessons_limit: 5,
            entities_limit: 5,
            entities_min_mentions: 3,
        }
    }
}

/// Phase 2A-4d.3.1 #3 — feature toggles for context injection.
///
/// Each boolean controls a specific class of injected content. Defaults
/// preserve today's behavior (everything `true`). Operators can flip any
/// toggle mid-session via `forge-next config set context_injection.<key>
/// <bool>`; handlers re-read the config on every request, so changes take
/// effect on the next hook call without daemon restart.
///
/// Toggles map to user-visible features, not hook-event timing:
///
/// * `session_context` — `<identity>`, `<disposition>`, `<decisions>`,
///   `<lessons>`, `<project-conventions>`, `<guardrails>` blocks in the
///   SessionStart `<forge-context>`.
/// * `active_state` — `<notifications>`, `<pending-messages>`,
///   `<meeting-context>`, `<perceptions>`, `<entities>` blocks (transient
///   state shown at session start + post-edit).
/// * `skills` — `<skills>` block in SessionStart **and** the
///   "Skill: Foo (procedural)" lines surfaced by pre-bash / post-edit
///   proactive injections.
/// * `anti_patterns` — "[proactive anti_pattern] ..." warnings surfaced
///   by pre-bash / post-edit / post-bash hooks. The loudest channel.
/// * `blast_radius` — post-edit caller / cluster warnings (callers_count,
///   calling_files, cluster_files in `BlastRadius` and `PostEditCheck`
///   responses).
/// * `preferences` — `<preferences>` and `<preferences-flipped>` blocks
///   in SessionStart context.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ContextInjectionConfig {
    #[serde(default = "default_true")]
    pub session_context: bool,
    #[serde(default = "default_true")]
    pub active_state: bool,
    #[serde(default = "default_true")]
    pub skills: bool,
    #[serde(default = "default_true")]
    pub anti_patterns: bool,
    #[serde(default = "default_true")]
    pub blast_radius: bool,
    #[serde(default = "default_true")]
    pub preferences: bool,
}

impl Default for ContextInjectionConfig {
    fn default() -> Self {
        Self {
            session_context: true,
            active_state: true,
            skills: true,
            anti_patterns: true,
            blast_radius: true,
            preferences: true,
        }
    }
}

/// Consolidation batch configuration — limits for consolidation phases.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ConsolidationConfig {
    #[serde(default = "default_200_usize")]
    pub batch_limit: usize,
    #[serde(default = "default_50_usize")]
    pub reweave_limit: usize,

    // Phase 23 behavioral skill inference (2A-4c2).
    #[serde(default = "default_3_usize")]
    pub skill_inference_min_sessions: usize,
    #[serde(default = "default_30_u32")]
    pub skill_inference_window_days: u32,
    #[serde(default = "default_1_f64")]
    pub skill_inference_tool_name_similarity_threshold: f64,
}

impl Default for ConsolidationConfig {
    fn default() -> Self {
        Self {
            batch_limit: 200,
            reweave_limit: 50,
            skill_inference_min_sessions: 3,
            skill_inference_window_days: 30,
            skill_inference_tool_name_similarity_threshold: 1.0,
        }
    }
}

/// Recall ranking configuration — boost factors for memory ranking.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RecallConfig {
    /// Recency boost for memories < 24h old
    pub recency_24h_boost: f64,
    /// Recency boost for memories < 7d old
    pub recency_7d_boost: f64,
    /// Access count boost threshold (high)
    pub access_high_threshold: i64,
    /// Access count boost factor (high)
    pub access_high_boost: f64,
    /// Access count boost threshold (medium)
    pub access_medium_threshold: i64,
    /// Access count boost factor (medium)
    pub access_medium_boost: f64,
    /// Domain DNA match boost factor
    pub domain_dna_boost: f64,
    /// Activation boost on recall
    pub activation_on_recall: f64,
    /// Activation boost on context inclusion
    pub activation_on_context: f64,
    /// Prefetch session recency weights
    pub prefetch_weights: Vec<f64>,
    /// Phase 2A-4b: half-life (in days) for preference recency multiplier.
    /// Used by recall.rs post-RRF and ops::decay_memories.
    /// Validated to 1.0..=365.0 in validated().
    pub preference_half_life_days: f64,
}

impl Default for RecallConfig {
    fn default() -> Self {
        Self {
            recency_24h_boost: 1.5,
            recency_7d_boost: 1.2,
            access_high_threshold: 10,
            access_high_boost: 1.3,
            access_medium_threshold: 3,
            access_medium_boost: 1.1,
            domain_dna_boost: 1.3,
            activation_on_recall: 0.3,
            activation_on_context: 0.1,
            prefetch_weights: vec![1.0, 0.7, 0.5],
            preference_half_life_days: 14.0,
        }
    }
}

impl RecallConfig {
    pub fn validated(&self) -> Self {
        Self {
            recency_24h_boost: self.recency_24h_boost.clamp(1.0, 5.0),
            recency_7d_boost: self.recency_7d_boost.clamp(1.0, 5.0),
            access_high_threshold: self.access_high_threshold.max(1),
            access_high_boost: self.access_high_boost.clamp(1.0, 5.0),
            access_medium_threshold: self.access_medium_threshold.max(1),
            access_medium_boost: self.access_medium_boost.clamp(1.0, 5.0),
            domain_dna_boost: self.domain_dna_boost.clamp(1.0, 5.0),
            activation_on_recall: self.activation_on_recall.clamp(0.0, 1.0),
            activation_on_context: self.activation_on_context.clamp(0.0, 1.0),
            prefetch_weights: if self.prefetch_weights.is_empty() {
                vec![1.0, 0.7, 0.5]
            } else {
                self.prefetch_weights.clone()
            },
            preference_half_life_days: self.preference_half_life_days.clamp(1.0, 365.0),
        }
    }
}

/// Reality Engine configuration — controls code intelligence features.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RealityConfig {
    #[serde(default = "default_true")]
    pub auto_detect: bool,
    #[serde(default = "default_false")]
    pub code_embeddings: bool,
    #[serde(default = "default_true")]
    pub community_detection: bool,
    #[serde(default = "default_5000_usize")]
    pub max_index_files: usize,
}

impl Default for RealityConfig {
    fn default() -> Self {
        Self {
            auto_detect: true,
            code_embeddings: false,
            community_detection: true,
            max_index_files: 5000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MeetingConfig {
    #[serde(default = "default_300_u64")]
    pub timeout_secs: u64,
    #[serde(default = "default_10_u64")]
    pub max_participants: u64,
}

impl Default for MeetingConfig {
    fn default() -> Self {
        Self {
            timeout_secs: 300,
            max_participants: 10,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentConfig {
    #[serde(default = "default_true")]
    pub auto_status: bool,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self { auto_status: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ProactiveConfig {
    #[serde(default = "default_300_usize")]
    pub refresh_budget_chars: usize,
    #[serde(default = "default_200_usize")]
    pub completion_check_budget_chars: usize,
    #[serde(default = "default_500_usize")]
    pub subagent_context_budget_chars: usize,
    #[serde(default = "default_anti_pattern_threshold")]
    pub anti_pattern_threshold: f64,
    #[serde(default = "default_completion_keywords")]
    pub completion_keywords: Vec<String>,
    #[serde(default = "default_3_usize")]
    pub completion_dismiss_limit: usize,
}

impl Default for ProactiveConfig {
    fn default() -> Self {
        Self {
            refresh_budget_chars: 300,
            completion_check_budget_chars: 200,
            subagent_context_budget_chars: 500,
            anti_pattern_threshold: 0.85,
            completion_keywords: default_completion_keywords(),
            completion_dismiss_limit: 3,
        }
    }
}

/// Memory Self-Healing configuration — thresholds for auto-supersede and staleness.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HealingConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_healing_cosine")]
    pub cosine_threshold: f64,
    #[serde(default = "default_healing_overlap_low")]
    pub overlap_low: f64,
    #[serde(default = "default_healing_overlap_high")]
    pub overlap_high: f64,
    #[serde(default = "default_healing_staleness_days")]
    pub staleness_days: u64,
    #[serde(default = "default_healing_staleness_min_quality")]
    pub staleness_min_quality: f64,
    #[serde(default = "default_healing_quality_decay")]
    pub quality_decay_per_cycle: f64,
    #[serde(default = "default_healing_quality_boost")]
    pub quality_boost_per_access: f64,
    #[serde(default = "default_200_usize")]
    pub batch_limit: usize,
}

impl Default for HealingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            cosine_threshold: 0.65,
            overlap_low: 0.3,
            overlap_high: 0.7,
            staleness_days: 7,
            staleness_min_quality: 0.2,
            quality_decay_per_cycle: 0.1,
            quality_boost_per_access: 0.05,
            batch_limit: 200,
        }
    }
}

/// Web UI static file serving — opt-in, disabled by default.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UiConfig {
    pub enabled: bool,
    pub dir: String,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            dir: "ui".to_string(),
        }
    }
}

/// Workspace configuration — controls how org/team directories are laid out.
/// Modes: "project" (default, no team dirs), "team" (inside .forge/teams/),
/// "distributed" (per-team roots), "centralized" (single PVC mount).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkspaceConfig {
    /// Workspace mode: "project" | "team" | "distributed" | "centralized"
    #[serde(default = "default_workspace_mode")]
    pub mode: String,
    /// Root directory for centralized mode (e.g., "/data/forge")
    #[serde(default)]
    pub root: String,
    /// Organization name for team/distributed modes
    #[serde(default)]
    pub org: String,
    /// Auto-write settings for workspace artifacts
    #[serde(default)]
    pub auto_write: AutoWriteConfig,
    /// Per-team workspace roots for distributed mode
    #[serde(default)]
    pub roots: HashMap<String, String>,
}

impl Default for WorkspaceConfig {
    fn default() -> Self {
        Self {
            mode: "project".to_string(),
            root: String::new(),
            org: String::new(),
            auto_write: AutoWriteConfig::default(),
            roots: HashMap::new(),
        }
    }
}

/// Controls which artifacts are automatically written to team workspaces.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AutoWriteConfig {
    /// Write decisions to team workspace
    pub decisions: bool,
    /// Write meeting minutes to team workspace
    pub meetings: bool,
    /// Write agent.json snapshots on spawn
    pub agent_snapshots: bool,
    /// Read team backlogs for context injection
    pub backlog_read: bool,
}

impl Default for AutoWriteConfig {
    fn default() -> Self {
        Self {
            decisions: true,
            meetings: true,
            agent_snapshots: true,
            backlog_read: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct A2aConfig {
    /// Whether A2A inter-session messaging is enabled at all.
    pub enabled: bool,
    /// Trust mode: "open" (default, all sessions can message freely) or "controlled" (check permission table).
    pub trust: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ExtractionConfig {
    pub backend: String, // "auto", "ollama", "claude", "claude_api", "openai", "gemini"
    pub routing: String, // "static" (default) or "smart"
    pub claude: ClaudeCliConfig,
    pub claude_api: ClaudeApiConfig,
    pub openai: OpenAiConfig,
    pub gemini: GeminiConfig,
    pub ollama: OllamaConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ClaudeCliConfig {
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ClaudeApiConfig {
    pub api_key: String, // or ANTHROPIC_API_KEY env var
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OpenAiConfig {
    pub api_key: String, // or OPENAI_API_KEY env var
    pub model: String,
    pub endpoint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GeminiConfig {
    pub api_key: String, // or GEMINI_API_KEY env var
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OllamaConfig {
    pub model: String,
    pub endpoint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EmbeddingConfig {
    pub model: String,
    pub dimensions: usize,
}

// ---------------------------------------------------------------------------
// Default impls
// ---------------------------------------------------------------------------

impl Default for ExtractionConfig {
    fn default() -> Self {
        Self {
            backend: "auto".to_string(),
            routing: "static".to_string(),
            claude: ClaudeCliConfig::default(),
            claude_api: ClaudeApiConfig::default(),
            openai: OpenAiConfig::default(),
            gemini: GeminiConfig::default(),
            ollama: OllamaConfig::default(),
        }
    }
}

impl Default for ClaudeCliConfig {
    fn default() -> Self {
        Self {
            model: "haiku".to_string(),
        }
    }
}

impl Default for ClaudeApiConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            model: "claude-haiku-4-5-20251001".to_string(),
        }
    }
}

impl Default for OpenAiConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            model: "gpt-4o-mini".to_string(),
            endpoint: "https://api.openai.com/v1".to_string(),
        }
    }
}

impl Default for GeminiConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            model: "gemini-2.0-flash".to_string(),
        }
    }
}

impl Default for OllamaConfig {
    fn default() -> Self {
        Self {
            model: "gemma3:1b".to_string(),
            endpoint: "http://localhost:11434".to_string(),
        }
    }
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            model: "nomic-embed-text".to_string(),
            dimensions: 768,
        }
    }
}

impl Default for A2aConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            trust: "open".to_string(),
        }
    }
}

impl WorkerConfig {
    /// Return a copy with all values clamped to sane bounds.
    pub fn validated(&self) -> Self {
        Self {
            extraction_debounce_secs: self.extraction_debounce_secs.max(1),
            consolidation_interval_secs: self.consolidation_interval_secs.clamp(60, 86400),
            embedding_interval_secs: self.embedding_interval_secs.clamp(10, 86400),
            perception_interval_secs: self.perception_interval_secs.clamp(5, 86400),
            disposition_interval_secs: self.disposition_interval_secs.clamp(60, 86400),
            indexer_interval_secs: self.indexer_interval_secs.clamp(60, 86400),
            diagnostics_debounce_secs: self.diagnostics_debounce_secs.max(1),
            session_reaper_interval_secs: self.session_reaper_interval_secs.clamp(10, 86400),
            heartbeat_timeout_secs: self.heartbeat_timeout_secs.clamp(10, 86400),
            // Phase 2A-4d.3.1 #7: idle threshold must be SHORTER than the
            // ended threshold to be reachable. 0 disables the idle phase
            // entirely. We clamp to (0, timeout) — values >= timeout
            // collapse to timeout-1, which still leaves a 1s idle window.
            heartbeat_idle_secs: if self.heartbeat_idle_secs == 0 {
                0
            } else {
                self.heartbeat_idle_secs
                    .clamp(1, self.heartbeat_timeout_secs.saturating_sub(1).max(1))
            },
            // Phase 2A-4d.2 T7: kpi_events retention reaper.
            // Clamp retention to 1..=365 days, reaper interval to 60s..=24h.
            kpi_events_retention_days: self.kpi_events_retention_days.clamp(1, 365),
            kpi_reaper_interval_secs: self.kpi_reaper_interval_secs.clamp(60, 86400),
            // Phase 2A-4d.3 T11 (D9): per-event-type retention overrides.
            // Clamp each non-zero entry to the same 1..=365 bounds as the
            // global value. T14 H5: drop entries with value `0` so they
            // fall through to the global default — operators were getting
            // 1-day retention from a silent clamp when they intended
            // "no per-type override / use the global". Dropping is the
            // least-surprising honest behavior.
            kpi_events_retention_days_by_type: self
                .kpi_events_retention_days_by_type
                .iter()
                .filter_map(|(k, v)| {
                    if *v == 0 {
                        tracing::warn!(
                            target: "forge::config",
                            event_type = %k,
                            "kpi_events_retention_days_by_type[{k}] = 0 dropped; \
                             global retention applies. Set ≥ 1 to override."
                        );
                        None
                    } else {
                        Some((k.clone(), (*v).clamp(1, 365)))
                    }
                })
                .collect(),
        }
    }
}

impl ContextConfig {
    /// Return a copy with all values clamped to sane bounds.
    pub fn validated(&self) -> Self {
        Self {
            budget_chars: self.budget_chars.clamp(256, 50000),
            decisions_limit: self.decisions_limit.clamp(1, 100),
            lessons_limit: self.lessons_limit.clamp(1, 100),
            entities_limit: self.entities_limit.clamp(0, 50),
            entities_min_mentions: self.entities_min_mentions.clamp(1, 100),
        }
    }
}

impl ConsolidationConfig {
    /// Return a copy with all values clamped to sane bounds.
    pub fn validated(&self) -> Self {
        Self {
            batch_limit: self.batch_limit.clamp(1, 1000),
            reweave_limit: self.reweave_limit.clamp(1, 500),
            skill_inference_min_sessions: self.skill_inference_min_sessions.clamp(1, 20),
            skill_inference_window_days: self.skill_inference_window_days.clamp(1, 365),
            skill_inference_tool_name_similarity_threshold: self
                .skill_inference_tool_name_similarity_threshold
                .clamp(0.0, 1.0),
        }
    }
}

impl ForgeConfig {
    /// Apply environment variable overrides to the config.
    /// Called AFTER loading config.toml so env vars take precedence.
    /// Invalid values (parse failures) are silently ignored — the config value remains unchanged.
    pub fn apply_env_overrides(&mut self) {
        // HTTP
        if let Ok(v) = std::env::var("FORGE_HTTP_ENABLED") {
            if let Ok(b) = v.parse::<bool>() {
                self.http.enabled = b;
            }
        }
        if let Ok(v) = std::env::var("FORGE_HTTP_BIND") {
            self.http.bind = v;
        }
        if let Ok(v) = std::env::var("FORGE_HTTP_PORT") {
            if let Ok(p) = v.parse::<u16>() {
                self.http.port = p;
            }
        }
        // gRPC
        if let Ok(v) = std::env::var("FORGE_GRPC_ENABLED") {
            if let Ok(b) = v.parse::<bool>() {
                self.grpc.enabled = b;
            }
        }
        if let Ok(v) = std::env::var("FORGE_GRPC_BIND") {
            self.grpc.bind = v;
        }
        if let Ok(v) = std::env::var("FORGE_GRPC_PORT") {
            if let Ok(p) = v.parse::<u16>() {
                self.grpc.port = p;
            }
        }
        // CORS
        if let Ok(v) = std::env::var("FORGE_CORS_ALLOWED_ORIGINS") {
            self.cors.allowed_origins = v.split(',').map(|s| s.trim().to_string()).collect();
        }
        if let Ok(v) = std::env::var("FORGE_CORS_MAX_AGE_SECS") {
            if let Ok(n) = v.parse::<u64>() {
                self.cors.max_age_secs = n;
            }
        }
        // Auth
        if let Ok(v) = std::env::var("FORGE_AUTH_ENABLED") {
            if let Ok(b) = v.parse::<bool>() {
                self.auth.enabled = b;
            }
        }
        if let Ok(v) = std::env::var("FORGE_AUTH_ISSUER_URL") {
            self.auth.issuer_url = v;
        }
        if let Ok(v) = std::env::var("FORGE_AUTH_AUDIENCE") {
            self.auth.audience = v;
        }
        if let Ok(v) = std::env::var("FORGE_AUTH_REQUIRED_CLAIMS") {
            self.auth.required_claims = v.split(',').map(|s| s.trim().to_string()).collect();
        }
        if let Ok(v) = std::env::var("FORGE_AUTH_ADMIN_EMAILS") {
            self.auth.admin_emails = v.split(',').map(|s| s.trim().to_string()).collect();
        }
        if let Ok(v) = std::env::var("FORGE_AUTH_VIEWER_EMAILS") {
            self.auth.viewer_emails = v.split(',').map(|s| s.trim().to_string()).collect();
        }
        if let Ok(v) = std::env::var("FORGE_AUTH_JWKS_CACHE_SECS") {
            if let Ok(n) = v.parse::<u64>() {
                self.auth.jwks_cache_secs = n;
            }
        }
        if let Ok(v) = std::env::var("FORGE_AUTH_OFFLINE_JWKS_PATH") {
            self.auth.offline_jwks_path = Some(v);
        }
        // Metrics
        if let Ok(v) = std::env::var("FORGE_METRICS_ENABLED") {
            if let Ok(b) = v.parse::<bool>() {
                self.metrics.enabled = b;
            }
        }
        // OTLP
        if let Ok(v) = std::env::var("FORGE_OTLP_ENABLED") {
            if let Ok(b) = v.parse::<bool>() {
                self.otlp.enabled = b;
            }
        }
        if let Ok(v) = std::env::var("FORGE_OTLP_ENDPOINT") {
            self.otlp.endpoint = v;
        }
        if let Ok(v) = std::env::var("FORGE_OTLP_SERVICE_NAME") {
            self.otlp.service_name = v;
        }
        // Session reaper / heartbeat
        if let Ok(v) = std::env::var("FORGE_SESSION_REAPER_INTERVAL") {
            if let Ok(n) = v.parse::<u64>() {
                self.workers.session_reaper_interval_secs = n;
            }
        }
        if let Ok(v) = std::env::var("FORGE_HEARTBEAT_TIMEOUT") {
            if let Ok(n) = v.parse::<u64>() {
                self.workers.heartbeat_timeout_secs = n;
            }
        }
    }

    /// Validate that config fields are sensible.
    pub fn validate(&self) -> Result<(), String> {
        if self.embedding.dimensions == 0 {
            return Err("embedding.dimensions must be > 0".into());
        }
        if self.extraction.claude.model.trim().is_empty() {
            return Err("extraction.claude.model must not be empty".into());
        }
        if self.extraction.ollama.model.trim().is_empty() {
            return Err("extraction.ollama.model must not be empty".into());
        }
        if self.extraction.ollama.endpoint.trim().is_empty() {
            return Err("extraction.ollama.endpoint must not be empty".into());
        }
        if !["open", "controlled"].contains(&self.a2a.trust.as_str()) {
            return Err(format!(
                "a2a.trust must be 'open' or 'controlled', got '{}'",
                self.a2a.trust
            ));
        }
        // HTTP validation
        if self.http.port == 0 {
            return Err("http.port must be > 0".into());
        }
        // gRPC validation
        if self.grpc.port == 0 {
            return Err("grpc.port must be > 0".into());
        }
        // Auth validation: if enabled, issuer_url and audience are required
        if self.auth.enabled {
            if self.auth.issuer_url.trim().is_empty() {
                return Err("auth.issuer_url must not be empty when auth is enabled".into());
            }
            if self.auth.audience.trim().is_empty() {
                return Err("auth.audience must not be empty when auth is enabled".into());
            }
        }
        // License tier validation
        let valid_tiers = ["free", "pro", "team", "enterprise"];
        if !valid_tiers.contains(&self.license.tier.as_str()) {
            return Err(format!(
                "license.tier must be one of {:?}, got '{}'",
                valid_tiers, self.license.tier
            ));
        }
        // ISS-D11: Security warning emitted only once (not on every config reload).
        if self.http.enabled
            && !self.auth.enabled
            && self.http.bind != "127.0.0.1"
            && self.http.bind != "localhost"
        {
            use std::sync::Once;
            static WARN_ONCE: Once = Once::new();
            WARN_ONCE.call_once(|| {
                eprintln!(
                    "[config] SECURITY WARNING: HTTP is bound to {} without auth enabled. \
                     The API is accessible to any network client without authentication. \
                     Set auth.enabled=true or bind to 127.0.0.1 for production.",
                    self.http.bind
                );
            });
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Loaders
// ---------------------------------------------------------------------------

/// Load config from `~/.forge/config.toml`.
/// Returns defaults if the file doesn't exist or can't be parsed.
pub fn load_config() -> ForgeConfig {
    let dir = forge_core::forge_dir();
    let path = format!("{dir}/config.toml");
    load_config_from(&path)
}

/// Load config from an arbitrary path.
/// Returns defaults if the file doesn't exist or can't be parsed.
pub fn load_config_from(path: &str) -> ForgeConfig {
    match std::fs::read_to_string(path) {
        Ok(contents) => {
            let config: ForgeConfig = match toml::from_str(&contents) {
                Ok(cfg) => cfg,
                Err(e) => {
                    eprintln!("forge: warning: failed to parse {path}: {e}");
                    return ForgeConfig::default();
                }
            };
            if let Err(e) = config.validate() {
                eprintln!("[config] validation error: {e}, using defaults");
                return ForgeConfig::default();
            }
            config
        }
        Err(_) => ForgeConfig::default(),
    }
}

// ---------------------------------------------------------------------------
// API key resolution
// ---------------------------------------------------------------------------

/// Resolve API key: config value > environment variable > None.
/// SECURITY: never log the returned key value.
pub fn resolve_api_key(config_value: &str, env_var: &str) -> Option<String> {
    if !config_value.is_empty() {
        return Some(config_value.to_string());
    }
    std::env::var(env_var).ok().filter(|k| !k.is_empty())
}

// ---------------------------------------------------------------------------
// Config update (persist changes to disk)
// ---------------------------------------------------------------------------

/// Update a config value by dotted key and persist to ~/.forge/config.toml.
pub fn update_config(key: &str, value: &str) -> Result<(), String> {
    let dir = forge_core::forge_dir();
    let path = format!("{dir}/config.toml");
    update_config_at(&path, key, value)
}

/// Update a config value at an arbitrary path (for testing).
pub fn update_config_at(path: &str, key: &str, value: &str) -> Result<(), String> {
    let content = std::fs::read_to_string(path).unwrap_or_default();
    // Parse into toml::Value to preserve existing keys without injecting defaults.
    // Then validate by applying the change to a typed ForgeConfig.
    let mut table: toml::Value =
        toml::from_str(&content).unwrap_or(toml::Value::Table(toml::map::Map::new()));
    let mut config: ForgeConfig = toml::from_str(&content).unwrap_or_default();

    match key.split('.').collect::<Vec<_>>().as_slice() {
        ["extraction", "backend"] => config.extraction.backend = value.to_string(),
        ["extraction", "claude", "model"] => config.extraction.claude.model = value.to_string(),
        ["extraction", "claude_api", "api_key"] => {
            config.extraction.claude_api.api_key = value.to_string()
        }
        ["extraction", "claude_api", "model"] => {
            config.extraction.claude_api.model = value.to_string()
        }
        ["extraction", "openai", "api_key"] => config.extraction.openai.api_key = value.to_string(),
        ["extraction", "openai", "model"] => config.extraction.openai.model = value.to_string(),
        ["extraction", "openai", "endpoint"] => {
            config.extraction.openai.endpoint = value.to_string()
        }
        ["extraction", "gemini", "api_key"] => config.extraction.gemini.api_key = value.to_string(),
        ["extraction", "gemini", "model"] => config.extraction.gemini.model = value.to_string(),
        ["extraction", "ollama", "model"] => config.extraction.ollama.model = value.to_string(),
        ["extraction", "ollama", "endpoint"] => {
            config.extraction.ollama.endpoint = value.to_string()
        }
        ["embedding", "model"] => config.embedding.model = value.to_string(),
        ["embedding", "dimensions"] => {
            config.embedding.dimensions = value
                .parse()
                .map_err(|e| format!("invalid dimensions: {e}"))?;
        }
        ["a2a", "enabled"] => {
            config.a2a.enabled = value
                .parse()
                .map_err(|e| format!("invalid a2a.enabled: {e}"))?;
        }
        ["a2a", "trust"] => {
            if !["open", "controlled"].contains(&value) {
                return Err(format!(
                    "a2a.trust must be 'open' or 'controlled', got '{value}'"
                ));
            }
            config.a2a.trust = value.to_string();
        }
        // Worker intervals
        ["workers", "extraction_debounce_secs"] => {
            config.workers.extraction_debounce_secs =
                value.parse().map_err(|e| format!("invalid value: {e}"))?;
        }
        ["workers", "consolidation_interval_secs"] => {
            config.workers.consolidation_interval_secs =
                value.parse().map_err(|e| format!("invalid value: {e}"))?;
        }
        ["workers", "embedding_interval_secs"] => {
            config.workers.embedding_interval_secs =
                value.parse().map_err(|e| format!("invalid value: {e}"))?;
        }
        ["workers", "perception_interval_secs"] => {
            config.workers.perception_interval_secs =
                value.parse().map_err(|e| format!("invalid value: {e}"))?;
        }
        ["workers", "disposition_interval_secs"] => {
            config.workers.disposition_interval_secs =
                value.parse().map_err(|e| format!("invalid value: {e}"))?;
        }
        ["workers", "indexer_interval_secs"] => {
            config.workers.indexer_interval_secs =
                value.parse().map_err(|e| format!("invalid value: {e}"))?;
        }
        ["workers", "diagnostics_debounce_secs"] => {
            config.workers.diagnostics_debounce_secs =
                value.parse().map_err(|e| format!("invalid value: {e}"))?;
        }
        ["workers", "session_reaper_interval_secs"] => {
            config.workers.session_reaper_interval_secs =
                value.parse().map_err(|e| format!("invalid value: {e}"))?;
        }
        ["workers", "heartbeat_timeout_secs"] => {
            config.workers.heartbeat_timeout_secs =
                value.parse().map_err(|e| format!("invalid value: {e}"))?;
        }
        ["workers", "kpi_events_retention_days"] => {
            config.workers.kpi_events_retention_days =
                value.parse().map_err(|e| format!("invalid value: {e}"))?;
        }
        ["workers", "kpi_reaper_interval_secs"] => {
            config.workers.kpi_reaper_interval_secs =
                value.parse().map_err(|e| format!("invalid value: {e}"))?;
        }
        // Context assembly
        ["context", "budget_chars"] => {
            config.context.budget_chars =
                value.parse().map_err(|e| format!("invalid value: {e}"))?;
        }
        ["context", "decisions_limit"] => {
            config.context.decisions_limit =
                value.parse().map_err(|e| format!("invalid value: {e}"))?;
        }
        ["context", "lessons_limit"] => {
            config.context.lessons_limit =
                value.parse().map_err(|e| format!("invalid value: {e}"))?;
        }
        ["context", "entities_limit"] => {
            config.context.entities_limit =
                value.parse().map_err(|e| format!("invalid value: {e}"))?;
        }
        ["context", "entities_min_mentions"] => {
            config.context.entities_min_mentions =
                value.parse().map_err(|e| format!("invalid value: {e}"))?;
        }
        // Consolidation
        ["consolidation", "batch_limit"] => {
            config.consolidation.batch_limit =
                value.parse().map_err(|e| format!("invalid value: {e}"))?;
        }
        ["consolidation", "reweave_limit"] => {
            config.consolidation.reweave_limit =
                value.parse().map_err(|e| format!("invalid value: {e}"))?;
        }
        // Recall boost factors
        ["recall", "recency_24h_boost"] => {
            config.recall.recency_24h_boost =
                value.parse().map_err(|e| format!("invalid value: {e}"))?;
        }
        ["recall", "recency_7d_boost"] => {
            config.recall.recency_7d_boost =
                value.parse().map_err(|e| format!("invalid value: {e}"))?;
        }
        ["recall", "access_high_threshold"] => {
            config.recall.access_high_threshold =
                value.parse().map_err(|e| format!("invalid value: {e}"))?;
        }
        ["recall", "access_high_boost"] => {
            config.recall.access_high_boost =
                value.parse().map_err(|e| format!("invalid value: {e}"))?;
        }
        ["recall", "access_medium_threshold"] => {
            config.recall.access_medium_threshold =
                value.parse().map_err(|e| format!("invalid value: {e}"))?;
        }
        ["recall", "access_medium_boost"] => {
            config.recall.access_medium_boost =
                value.parse().map_err(|e| format!("invalid value: {e}"))?;
        }
        ["recall", "domain_dna_boost"] => {
            config.recall.domain_dna_boost =
                value.parse().map_err(|e| format!("invalid value: {e}"))?;
        }
        ["recall", "activation_on_recall"] => {
            config.recall.activation_on_recall =
                value.parse().map_err(|e| format!("invalid value: {e}"))?;
        }
        ["recall", "activation_on_context"] => {
            config.recall.activation_on_context =
                value.parse().map_err(|e| format!("invalid value: {e}"))?;
        }
        // Reality Engine
        ["reality", "auto_detect"] => {
            config.reality.auto_detect =
                value.parse().map_err(|e| format!("invalid value: {e}"))?;
        }
        ["reality", "code_embeddings"] => {
            config.reality.code_embeddings =
                value.parse().map_err(|e| format!("invalid value: {e}"))?;
        }
        ["reality", "community_detection"] => {
            config.reality.community_detection =
                value.parse().map_err(|e| format!("invalid value: {e}"))?;
        }
        ["reality", "max_index_files"] => {
            let v: usize = value.parse().map_err(|e| format!("invalid value: {e}"))?;
            config.reality.max_index_files = v.clamp(100, 50000);
        }
        // Meeting
        ["meeting", "timeout_secs"] => {
            config.meeting.timeout_secs =
                value.parse().map_err(|e| format!("invalid value: {e}"))?;
        }
        ["meeting", "max_participants"] => {
            config.meeting.max_participants =
                value.parse().map_err(|e| format!("invalid value: {e}"))?;
        }
        // Agent
        ["agent", "auto_status"] => {
            config.agent.auto_status = value.parse().map_err(|e| format!("invalid value: {e}"))?;
        }
        // OTLP
        ["otlp", "enabled"] => {
            config.otlp.enabled = value.parse().map_err(|e| format!("invalid value: {e}"))?;
        }
        ["otlp", "endpoint"] => {
            config.otlp.endpoint = value.to_string();
        }
        ["otlp", "service_name"] => {
            config.otlp.service_name = value.to_string();
        }
        // License
        ["license", "tier"] => {
            let valid = ["free", "pro", "team", "enterprise"];
            if !valid.contains(&value) {
                return Err(format!(
                    "license.tier must be one of {valid:?}, got '{value}'"
                ));
            }
            config.license.tier = value.to_string();
        }
        ["license", "key"] => {
            config.license.key = value.to_string();
        }
        _ => return Err(format!("unknown config key: {key}")),
    }

    // Surgically update only the changed key in the TOML table,
    // preserving all other user settings (ISS-9 fix).
    // Determine the correct typed TOML value by comparing old vs new config.
    let parts: Vec<&str> = key.split('.').collect();
    let toml_table = table.as_table_mut().ok_or("config is not a TOML table")?;

    // Derive TOML value type from the typed config field (not string guessing).
    // The match arms above already validated and set the correct Rust type on `config`.
    // Serialize the typed config, extract just the changed field's TOML value.
    let typed_toml: toml::Value = toml::to_string_pretty(&config)
        .ok()
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or(toml::Value::Table(toml::map::Map::new()));
    let toml_val: toml::Value = match parts.as_slice() {
        [section, field] => typed_toml
            .get(*section)
            .and_then(|s| s.get(*field))
            .cloned()
            .unwrap_or_else(|| toml::Value::String(value.to_string())),
        [section, subsection, field] => typed_toml
            .get(*section)
            .and_then(|s| s.get(*subsection))
            .and_then(|s| s.get(*field))
            .cloned()
            .unwrap_or_else(|| toml::Value::String(value.to_string())),
        _ => toml::Value::String(value.to_string()),
    };

    match parts.as_slice() {
        [section, field] => {
            let section_table = toml_table
                .entry(*section)
                .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
            if let Some(t) = section_table.as_table_mut() {
                t.insert(field.to_string(), toml_val);
            }
        }
        [section, subsection, field] => {
            let section_table = toml_table
                .entry(*section)
                .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
            if let Some(st) = section_table.as_table_mut() {
                let sub_table = st
                    .entry(*subsection)
                    .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
                if let Some(subt) = sub_table.as_table_mut() {
                    subt.insert(field.to_string(), toml_val);
                }
            }
        }
        _ => return Err(format!("unsupported key depth: {key}")),
    }

    let toml_str = toml::to_string_pretty(&table).map_err(|e| format!("serialize error: {e}"))?;
    std::fs::write(path, toml_str).map_err(|e| format!("write error: {e}"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    #[test]
    fn test_default_config() {
        let cfg = ForgeConfig::default();

        // Extraction defaults
        assert_eq!(cfg.extraction.backend, "auto");
        assert_eq!(cfg.extraction.claude.model, "haiku");
        assert_eq!(cfg.extraction.ollama.model, "gemma3:1b");
        assert_eq!(cfg.extraction.ollama.endpoint, "http://localhost:11434");

        // Embedding defaults
        assert_eq!(cfg.embedding.model, "nomic-embed-text");
        assert_eq!(cfg.embedding.dimensions, 768);

        // Phase 2A-4d.3.1 #3 — context_injection defaults preserve today's
        // behavior (everything injected). Operators opt out via SetConfig.
        let inj = &cfg.context_injection;
        assert!(inj.session_context, "session_context should default true");
        assert!(inj.active_state, "active_state should default true");
        assert!(inj.skills, "skills should default true");
        assert!(inj.anti_patterns, "anti_patterns should default true");
        assert!(inj.blast_radius, "blast_radius should default true");
        assert!(inj.preferences, "preferences should default true");
    }

    #[test]
    fn test_context_injection_partial_toml_keeps_unspecified_defaults() {
        // Operators flipping a single toggle via TOML (e.g., daemon config
        // file) must NOT lose other defaults. #[serde(default)] on the
        // struct ensures missing fields fall back to true.
        let toml_str = r#"
[context_injection]
anti_patterns = false
"#;
        let cfg: ForgeConfig = toml::from_str(toml_str).expect("parses");
        assert!(!cfg.context_injection.anti_patterns, "explicit false");
        assert!(cfg.context_injection.session_context, "default preserved");
        assert!(cfg.context_injection.skills, "default preserved");
        assert!(cfg.context_injection.blast_radius, "default preserved");
    }

    #[test]
    fn test_context_injection_all_off_toml() {
        let toml_str = r#"
[context_injection]
session_context = false
active_state = false
skills = false
anti_patterns = false
blast_radius = false
preferences = false
"#;
        let cfg: ForgeConfig = toml::from_str(toml_str).expect("parses");
        let inj = &cfg.context_injection;
        assert!(!inj.session_context);
        assert!(!inj.active_state);
        assert!(!inj.skills);
        assert!(!inj.anti_patterns);
        assert!(!inj.blast_radius);
        assert!(!inj.preferences);
    }

    #[test]
    fn test_parse_config_toml() {
        let toml_str = r#"
[extraction]
backend = "claude"

[extraction.claude]
model = "sonnet"

[extraction.ollama]
model = "llama3:70b"
endpoint = "http://gpu-server:11434"

[embedding]
model = "mxbai-embed-large"
dimensions = 1024
"#;

        let cfg: ForgeConfig = toml::from_str(toml_str).unwrap();

        assert_eq!(cfg.extraction.backend, "claude");
        assert_eq!(cfg.extraction.claude.model, "sonnet");
        assert_eq!(cfg.extraction.ollama.model, "llama3:70b");
        assert_eq!(cfg.extraction.ollama.endpoint, "http://gpu-server:11434");
        assert_eq!(cfg.embedding.model, "mxbai-embed-large");
        assert_eq!(cfg.embedding.dimensions, 1024);
    }

    #[test]
    fn test_partial_config() {
        let toml_str = r#"
[extraction]
backend = "ollama"
"#;

        let cfg: ForgeConfig = toml::from_str(toml_str).unwrap();

        // Overridden field
        assert_eq!(cfg.extraction.backend, "ollama");

        // All other fields should be defaults
        assert_eq!(cfg.extraction.claude.model, "haiku");
        assert_eq!(cfg.extraction.ollama.model, "gemma3:1b");
        assert_eq!(cfg.extraction.ollama.endpoint, "http://localhost:11434");
        assert_eq!(cfg.embedding.model, "nomic-embed-text");
        assert_eq!(cfg.embedding.dimensions, 768);
    }

    #[test]
    fn test_validate_zero_dimensions() {
        let mut config = ForgeConfig::default();
        config.embedding.dimensions = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_empty_model() {
        let mut config = ForgeConfig::default();
        config.extraction.claude.model = "".into();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_default_passes() {
        let config = ForgeConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_missing_file_returns_defaults() {
        let cfg = load_config_from("/nonexistent/path/config.toml");

        assert_eq!(cfg.extraction.backend, "auto");
        assert_eq!(cfg.extraction.claude.model, "haiku");
        assert_eq!(cfg.extraction.ollama.model, "gemma3:1b");
        assert_eq!(cfg.extraction.ollama.endpoint, "http://localhost:11434");
        assert_eq!(cfg.embedding.model, "nomic-embed-text");
        assert_eq!(cfg.embedding.dimensions, 768);
    }

    #[test]
    fn test_new_provider_defaults() {
        let cfg = ForgeConfig::default();

        // Claude API defaults
        assert!(cfg.extraction.claude_api.api_key.is_empty());
        assert_eq!(cfg.extraction.claude_api.model, "claude-haiku-4-5-20251001");

        // OpenAI defaults
        assert!(cfg.extraction.openai.api_key.is_empty());
        assert_eq!(cfg.extraction.openai.model, "gpt-4o-mini");
        assert_eq!(cfg.extraction.openai.endpoint, "https://api.openai.com/v1");

        // Gemini defaults
        assert!(cfg.extraction.gemini.api_key.is_empty());
        assert_eq!(cfg.extraction.gemini.model, "gemini-2.0-flash");
    }

    #[test]
    fn test_parse_config_with_new_providers() {
        let toml_str = r#"
[extraction]
backend = "claude_api"

[extraction.claude_api]
api_key = "sk-ant-test"
model = "claude-sonnet-4-20250514"

[extraction.openai]
api_key = "sk-openai-test"
model = "gpt-4o"
endpoint = "https://custom.openai.com/v1"

[extraction.gemini]
api_key = "gemini-test-key"
model = "gemini-1.5-pro"
"#;

        let cfg: ForgeConfig = toml::from_str(toml_str).unwrap();

        assert_eq!(cfg.extraction.backend, "claude_api");
        assert_eq!(cfg.extraction.claude_api.api_key, "sk-ant-test");
        assert_eq!(cfg.extraction.claude_api.model, "claude-sonnet-4-20250514");
        assert_eq!(cfg.extraction.openai.api_key, "sk-openai-test");
        assert_eq!(cfg.extraction.openai.model, "gpt-4o");
        assert_eq!(
            cfg.extraction.openai.endpoint,
            "https://custom.openai.com/v1"
        );
        assert_eq!(cfg.extraction.gemini.api_key, "gemini-test-key");
        assert_eq!(cfg.extraction.gemini.model, "gemini-1.5-pro");
    }

    #[test]
    fn test_resolve_api_key_from_config() {
        // Config value takes priority over env var
        let result = resolve_api_key("sk-from-config", "NONEXISTENT_VAR_12345");
        assert_eq!(result, Some("sk-from-config".to_string()));
    }

    #[test]
    fn test_resolve_api_key_from_env() {
        // Set a temporary env var
        std::env::set_var("FORGE_TEST_API_KEY_12345", "sk-from-env");
        let result = resolve_api_key("", "FORGE_TEST_API_KEY_12345");
        assert_eq!(result, Some("sk-from-env".to_string()));
        std::env::remove_var("FORGE_TEST_API_KEY_12345");
    }

    #[test]
    fn test_resolve_api_key_none() {
        // Neither config nor env var set
        let result = resolve_api_key("", "NONEXISTENT_VAR_67890");
        assert_eq!(result, None);
    }

    #[test]
    fn test_resolve_api_key_empty_env() {
        // Empty env var should return None
        std::env::set_var("FORGE_TEST_EMPTY_KEY", "");
        let result = resolve_api_key("", "FORGE_TEST_EMPTY_KEY");
        assert_eq!(result, None);
        std::env::remove_var("FORGE_TEST_EMPTY_KEY");
    }

    #[test]
    fn test_config_reload_from_disk() {
        // Write initial config to temp file
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let path_str = path.to_str().unwrap();

        let initial_toml = r#"
[extraction]
backend = "ollama"

[extraction.ollama]
model = "gemma3:1b"
"#;
        std::fs::write(&path, initial_toml).unwrap();

        // Load initial config
        let cfg1 = load_config_from(path_str);
        assert_eq!(cfg1.extraction.backend, "ollama");
        assert_eq!(cfg1.extraction.ollama.model, "gemma3:1b");

        // Change config on disk (simulates `forge-next config set`)
        let updated_toml = r#"
[extraction]
backend = "claude_api"

[extraction.ollama]
model = "llama3:70b"
"#;
        std::fs::write(&path, updated_toml).unwrap();

        // Reload — should see new values without restart
        let cfg2 = load_config_from(path_str);
        assert_eq!(
            cfg2.extraction.backend, "claude_api",
            "backend should reflect disk change"
        );
        assert_eq!(
            cfg2.extraction.ollama.model, "llama3:70b",
            "model should reflect disk change"
        );
    }

    #[test]
    fn test_config_defaults_match_current() {
        // Verify that all new config defaults match the previously hardcoded values.
        // This is the critical zero-behavioral-change guarantee.
        let cfg = ForgeConfig::default();

        // Worker intervals
        assert_eq!(
            cfg.workers.extraction_debounce_secs, 15,
            "extraction debounce was 15s"
        );
        assert_eq!(
            cfg.workers.consolidation_interval_secs, 1800,
            "consolidation was 30*60=1800s"
        );
        assert_eq!(cfg.workers.embedding_interval_secs, 60, "embedder was 60s");
        assert_eq!(
            cfg.workers.perception_interval_secs, 30,
            "perception was 30s"
        );
        assert_eq!(
            cfg.workers.disposition_interval_secs, 900,
            "disposition was 15*60=900s"
        );
        assert_eq!(
            cfg.workers.indexer_interval_secs, 300,
            "indexer was 5*60=300s"
        );
        assert_eq!(
            cfg.workers.diagnostics_debounce_secs, 3,
            "diagnostics debounce was 3s"
        );

        // Context assembly
        assert_eq!(cfg.context.budget_chars, 3000, "budget was hardcoded 3000");
        assert_eq!(cfg.context.decisions_limit, 10, "decisions LIMIT was 10");
        assert_eq!(cfg.context.lessons_limit, 5, "lessons LIMIT was 5");
        assert_eq!(cfg.context.entities_limit, 5, "entities limit was 5");
        assert_eq!(
            cfg.context.entities_min_mentions, 3,
            "entities min mentions was >= 3"
        );

        // Consolidation
        assert_eq!(
            cfg.consolidation.batch_limit, 200,
            "consolidation LIMIT was 200"
        );
        assert_eq!(cfg.consolidation.reweave_limit, 50, "reweave limit was 50");
    }

    #[test]
    fn test_config_roundtrip() {
        // Serialize and deserialize with new sections to verify TOML roundtrip.
        let mut cfg = ForgeConfig::default();
        cfg.workers.consolidation_interval_secs = 600;
        cfg.context.decisions_limit = 20;
        cfg.consolidation.batch_limit = 100;

        let toml_str = toml::to_string_pretty(&cfg).unwrap();
        let parsed: ForgeConfig = toml::from_str(&toml_str).unwrap();

        assert_eq!(parsed.workers.consolidation_interval_secs, 600);
        assert_eq!(parsed.context.decisions_limit, 20);
        assert_eq!(parsed.consolidation.batch_limit, 100);
        // Other defaults should be preserved
        assert_eq!(parsed.workers.extraction_debounce_secs, 15);
        assert_eq!(parsed.context.budget_chars, 3000);
        assert_eq!(parsed.consolidation.reweave_limit, 50);
    }

    #[test]
    fn test_config_partial_toml_with_new_sections() {
        // Verify backward compatibility: a config.toml that doesn't mention
        // workers/context/consolidation should use all defaults.
        let toml_str = r#"
[extraction]
backend = "ollama"
"#;
        let cfg: ForgeConfig = toml::from_str(toml_str).unwrap();

        assert_eq!(cfg.extraction.backend, "ollama");
        // New sections should all be defaults
        assert_eq!(cfg.workers.extraction_debounce_secs, 15);
        assert_eq!(cfg.workers.consolidation_interval_secs, 1800);
        assert_eq!(cfg.context.budget_chars, 3000);
        assert_eq!(cfg.context.decisions_limit, 10);
        assert_eq!(cfg.consolidation.batch_limit, 200);
    }

    #[test]
    fn test_config_set_get_new_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let path_str = path.to_str().unwrap();

        // Start with default config
        std::fs::write(&path, "").unwrap();

        // Set worker interval
        update_config_at(path_str, "workers.consolidation_interval_secs", "600").unwrap();
        let cfg = load_config_from(path_str);
        assert_eq!(cfg.workers.consolidation_interval_secs, 600);

        // Set context limit
        update_config_at(path_str, "context.decisions_limit", "20").unwrap();
        let cfg = load_config_from(path_str);
        assert_eq!(cfg.context.decisions_limit, 20);
        // Previous update should be preserved
        assert_eq!(cfg.workers.consolidation_interval_secs, 600);

        // Set consolidation limit
        update_config_at(path_str, "consolidation.batch_limit", "100").unwrap();
        let cfg = load_config_from(path_str);
        assert_eq!(cfg.consolidation.batch_limit, 100);

        // Set all worker keys
        update_config_at(path_str, "workers.extraction_debounce_secs", "30").unwrap();
        update_config_at(path_str, "workers.embedding_interval_secs", "120").unwrap();
        update_config_at(path_str, "workers.perception_interval_secs", "60").unwrap();
        update_config_at(path_str, "workers.disposition_interval_secs", "1800").unwrap();
        update_config_at(path_str, "workers.indexer_interval_secs", "600").unwrap();
        update_config_at(path_str, "workers.diagnostics_debounce_secs", "5").unwrap();
        let cfg = load_config_from(path_str);
        assert_eq!(cfg.workers.extraction_debounce_secs, 30);
        assert_eq!(cfg.workers.embedding_interval_secs, 120);
        assert_eq!(cfg.workers.perception_interval_secs, 60);
        assert_eq!(cfg.workers.disposition_interval_secs, 1800);
        assert_eq!(cfg.workers.indexer_interval_secs, 600);
        assert_eq!(cfg.workers.diagnostics_debounce_secs, 5);

        // Set all context keys
        update_config_at(path_str, "context.budget_chars", "5000").unwrap();
        update_config_at(path_str, "context.lessons_limit", "10").unwrap();
        update_config_at(path_str, "context.entities_limit", "8").unwrap();
        update_config_at(path_str, "context.entities_min_mentions", "5").unwrap();
        let cfg = load_config_from(path_str);
        assert_eq!(cfg.context.budget_chars, 5000);
        assert_eq!(cfg.context.lessons_limit, 10);
        assert_eq!(cfg.context.entities_limit, 8);
        assert_eq!(cfg.context.entities_min_mentions, 5);

        // Set all consolidation keys
        update_config_at(path_str, "consolidation.reweave_limit", "25").unwrap();
        let cfg = load_config_from(path_str);
        assert_eq!(cfg.consolidation.reweave_limit, 25);

        // Invalid value should error
        let err = update_config_at(
            path_str,
            "workers.consolidation_interval_secs",
            "not_a_number",
        );
        assert!(err.is_err());
    }

    #[test]
    fn test_update_config_preserves_unrelated_keys() {
        // ISS-9 regression test: setting license should NOT clear skills_directory
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let path_str = path.to_str().unwrap();

        // Write a config with skills_directory set (top-level field)
        std::fs::write(
            &path,
            r#"
skills_directory = "product/claude-skills"

[extraction]
backend = "gemini"
"#,
        )
        .unwrap();

        // Now update license — this MUST NOT clear skills_directory
        update_config_at(path_str, "license.tier", "pro").unwrap();

        // Verify skills_directory is preserved
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(
            content.contains("product/claude-skills"),
            "skills_directory should be preserved after license update, got:\n{content}"
        );
        assert!(
            content.contains("gemini"),
            "extraction backend should be preserved after license update, got:\n{content}"
        );

        // Also verify the license was actually set
        let cfg = load_config_from(path_str);
        assert_eq!(cfg.license.tier, "pro");
        assert_eq!(cfg.skills_directory, "product/claude-skills");
        assert_eq!(cfg.extraction.backend, "gemini");
    }

    #[test]
    fn test_reality_config_defaults() {
        let cfg = ForgeConfig::default();
        assert!(
            cfg.reality.auto_detect,
            "auto_detect default should be true"
        );
        assert!(
            !cfg.reality.code_embeddings,
            "code_embeddings default should be false"
        );
        assert!(
            cfg.reality.community_detection,
            "community_detection default should be true"
        );
        assert_eq!(
            cfg.reality.max_index_files, 5000,
            "max_index_files default should be 5000"
        );
    }

    // -----------------------------------------------------------------------
    // Enterprise config tests (HttpConfig, CorsConfig, AuthConfig)
    // -----------------------------------------------------------------------

    #[test]
    fn test_http_config_defaults() {
        let cfg = HttpConfig::default();
        assert!(!cfg.enabled, "http.enabled default should be false");
        assert_eq!(
            cfg.bind, "127.0.0.1",
            "http.bind default should be 127.0.0.1"
        );
        assert_eq!(cfg.port, 8420, "http.port default should be 8420");
    }

    #[test]
    fn test_cors_config_defaults() {
        let cfg = CorsConfig::default();
        let expected_origins = vec![
            "http://localhost:*".to_string(),
            "https://localhost:*".to_string(),
            "http://127.0.0.1:*".to_string(),
            "https://127.0.0.1:*".to_string(),
        ];
        assert_eq!(
            cfg.allowed_origins, expected_origins,
            "cors.allowed_origins default should be localhost-only"
        );
        assert_eq!(
            cfg.max_age_secs, 3600,
            "cors.max_age_secs default should be 3600"
        );
    }

    #[test]
    fn test_auth_config_defaults() {
        let cfg = AuthConfig::default();
        assert!(!cfg.enabled, "auth.enabled default should be false");
        assert!(
            cfg.issuer_url.is_empty(),
            "auth.issuer_url default should be empty"
        );
        assert!(
            cfg.audience.is_empty(),
            "auth.audience default should be empty"
        );
        assert!(
            cfg.required_claims.is_empty(),
            "auth.required_claims default should be empty"
        );
        assert!(
            cfg.admin_emails.is_empty(),
            "auth.admin_emails default should be empty"
        );
        assert_eq!(
            cfg.jwks_cache_secs, 3600,
            "auth.jwks_cache_secs default should be 3600"
        );
        assert!(
            cfg.offline_jwks_path.is_none(),
            "auth.offline_jwks_path default should be None"
        );
    }

    #[test]
    fn test_forge_config_has_enterprise_sections() {
        let cfg = ForgeConfig::default();
        // Verify enterprise sections exist and have defaults
        assert!(!cfg.http.enabled);
        assert_eq!(cfg.http.port, 8420);
        assert!(!cfg.auth.enabled);
        assert_eq!(
            cfg.cors.allowed_origins.len(),
            4,
            "CORS should have 4 localhost origins"
        );
        assert!(
            cfg.cors.allowed_origins[0].starts_with("http://localhost"),
            "first origin should be http://localhost"
        );
    }

    #[test]
    fn test_enterprise_config_roundtrip() {
        let mut cfg = ForgeConfig::default();
        cfg.http.enabled = true;
        cfg.http.port = 9090;
        cfg.http.bind = "0.0.0.0".to_string();
        cfg.cors.allowed_origins = vec!["https://app.example.com".to_string()];
        cfg.cors.max_age_secs = 7200;
        cfg.auth.enabled = true;
        cfg.auth.issuer_url = "https://accounts.google.com".to_string();
        cfg.auth.audience = "my-forge-app".to_string();
        cfg.auth.required_claims = vec!["email".to_string(), "sub".to_string()];
        cfg.auth.admin_emails = vec!["admin@example.com".to_string()];
        cfg.auth.jwks_cache_secs = 1800;
        cfg.auth.offline_jwks_path = Some("/path/to/jwks.json".to_string());

        let toml_str = toml::to_string_pretty(&cfg).unwrap();
        let parsed: ForgeConfig = toml::from_str(&toml_str).unwrap();

        assert!(parsed.http.enabled);
        assert_eq!(parsed.http.port, 9090);
        assert_eq!(parsed.http.bind, "0.0.0.0");
        assert_eq!(
            parsed.cors.allowed_origins,
            vec!["https://app.example.com".to_string()]
        );
        assert_eq!(parsed.cors.max_age_secs, 7200);
        assert!(parsed.auth.enabled);
        assert_eq!(parsed.auth.issuer_url, "https://accounts.google.com");
        assert_eq!(parsed.auth.audience, "my-forge-app");
        assert_eq!(parsed.auth.required_claims, vec!["email", "sub"]);
        assert_eq!(parsed.auth.admin_emails, vec!["admin@example.com"]);
        assert_eq!(parsed.auth.jwks_cache_secs, 1800);
        assert_eq!(
            parsed.auth.offline_jwks_path,
            Some("/path/to/jwks.json".to_string())
        );
    }

    #[test]
    fn test_enterprise_config_partial_toml() {
        // Old config.toml without enterprise sections should still work
        let toml_str = r#"
[extraction]
backend = "ollama"
"#;
        let cfg: ForgeConfig = toml::from_str(toml_str).unwrap();
        assert!(!cfg.http.enabled);
        assert_eq!(cfg.http.port, 8420);
        assert!(!cfg.auth.enabled);
        assert_eq!(
            cfg.cors.allowed_origins.len(),
            4,
            "CORS should default to localhost-only origins"
        );
    }

    #[test]
    fn test_enterprise_config_from_toml() {
        let toml_str = r#"
[http]
enabled = true
bind = "0.0.0.0"
port = 9090

[cors]
allowed_origins = ["https://app.example.com", "https://admin.example.com"]
max_age_secs = 7200

[auth]
enabled = true
issuer_url = "https://accounts.google.com"
audience = "forge-prod"
required_claims = ["email"]
admin_emails = ["admin@example.com"]
jwks_cache_secs = 1800
offline_jwks_path = "/etc/forge/jwks.json"
"#;
        let cfg: ForgeConfig = toml::from_str(toml_str).unwrap();
        assert!(cfg.http.enabled);
        assert_eq!(cfg.http.bind, "0.0.0.0");
        assert_eq!(cfg.http.port, 9090);
        assert_eq!(
            cfg.cors.allowed_origins,
            vec!["https://app.example.com", "https://admin.example.com"]
        );
        assert_eq!(cfg.cors.max_age_secs, 7200);
        assert!(cfg.auth.enabled);
        assert_eq!(cfg.auth.issuer_url, "https://accounts.google.com");
        assert_eq!(cfg.auth.audience, "forge-prod");
        assert_eq!(cfg.auth.required_claims, vec!["email"]);
        assert_eq!(cfg.auth.admin_emails, vec!["admin@example.com"]);
        assert_eq!(cfg.auth.jwks_cache_secs, 1800);
        assert_eq!(
            cfg.auth.offline_jwks_path,
            Some("/etc/forge/jwks.json".to_string())
        );
    }

    #[test]
    fn test_validate_http_port_zero() {
        let mut cfg = ForgeConfig::default();
        cfg.http.port = 0;
        assert!(cfg.validate().is_err(), "port 0 should fail validation");
    }

    #[test]
    fn test_validate_auth_enabled_without_issuer() {
        let mut cfg = ForgeConfig::default();
        cfg.auth.enabled = true;
        cfg.auth.audience = "test".to_string();
        // issuer_url is empty - should fail
        assert!(
            cfg.validate().is_err(),
            "auth.enabled without issuer_url should fail"
        );
    }

    #[test]
    fn test_validate_auth_enabled_without_audience() {
        let mut cfg = ForgeConfig::default();
        cfg.auth.enabled = true;
        cfg.auth.issuer_url = "https://issuer.example.com".to_string();
        // audience is empty - should fail
        assert!(
            cfg.validate().is_err(),
            "auth.enabled without audience should fail"
        );
    }

    #[test]
    fn test_validate_auth_disabled_allows_empty_fields() {
        let cfg = ForgeConfig::default();
        // auth.enabled=false, empty issuer/audience should be fine
        assert!(cfg.validate().is_ok());
    }

    #[test]
    #[serial]
    fn test_env_override_http() {
        let mut cfg = ForgeConfig::default();
        std::env::set_var("FORGE_HTTP_ENABLED", "true");
        std::env::set_var("FORGE_HTTP_BIND", "0.0.0.0");
        std::env::set_var("FORGE_HTTP_PORT", "9090");

        cfg.apply_env_overrides();

        assert!(cfg.http.enabled);
        assert_eq!(cfg.http.bind, "0.0.0.0");
        assert_eq!(cfg.http.port, 9090);

        std::env::remove_var("FORGE_HTTP_ENABLED");
        std::env::remove_var("FORGE_HTTP_BIND");
        std::env::remove_var("FORGE_HTTP_PORT");
    }

    #[test]
    #[serial]
    fn test_env_override_cors() {
        let mut cfg = ForgeConfig::default();
        std::env::set_var("FORGE_CORS_ALLOWED_ORIGINS", "https://a.com,https://b.com");
        std::env::set_var("FORGE_CORS_MAX_AGE_SECS", "7200");

        cfg.apply_env_overrides();

        assert_eq!(
            cfg.cors.allowed_origins,
            vec!["https://a.com", "https://b.com"]
        );
        assert_eq!(cfg.cors.max_age_secs, 7200);

        std::env::remove_var("FORGE_CORS_ALLOWED_ORIGINS");
        std::env::remove_var("FORGE_CORS_MAX_AGE_SECS");
    }

    #[test]
    #[serial]
    fn test_env_override_auth() {
        let mut cfg = ForgeConfig::default();
        std::env::set_var("FORGE_AUTH_ENABLED", "true");
        std::env::set_var("FORGE_AUTH_ISSUER_URL", "https://issuer.example.com");
        std::env::set_var("FORGE_AUTH_AUDIENCE", "my-app");
        std::env::set_var("FORGE_AUTH_REQUIRED_CLAIMS", "email,sub");
        std::env::set_var("FORGE_AUTH_ADMIN_EMAILS", "admin@test.com,boss@test.com");
        std::env::set_var("FORGE_AUTH_JWKS_CACHE_SECS", "1800");
        std::env::set_var("FORGE_AUTH_OFFLINE_JWKS_PATH", "/tmp/jwks.json");

        cfg.apply_env_overrides();

        assert!(cfg.auth.enabled);
        assert_eq!(cfg.auth.issuer_url, "https://issuer.example.com");
        assert_eq!(cfg.auth.audience, "my-app");
        assert_eq!(cfg.auth.required_claims, vec!["email", "sub"]);
        assert_eq!(
            cfg.auth.admin_emails,
            vec!["admin@test.com", "boss@test.com"]
        );
        assert_eq!(cfg.auth.jwks_cache_secs, 1800);
        assert_eq!(
            cfg.auth.offline_jwks_path,
            Some("/tmp/jwks.json".to_string())
        );

        std::env::remove_var("FORGE_AUTH_ENABLED");
        std::env::remove_var("FORGE_AUTH_ISSUER_URL");
        std::env::remove_var("FORGE_AUTH_AUDIENCE");
        std::env::remove_var("FORGE_AUTH_REQUIRED_CLAIMS");
        std::env::remove_var("FORGE_AUTH_ADMIN_EMAILS");
        std::env::remove_var("FORGE_AUTH_JWKS_CACHE_SECS");
        std::env::remove_var("FORGE_AUTH_OFFLINE_JWKS_PATH");
    }

    #[test]
    #[serial]
    fn test_env_override_no_env_vars_set() {
        let mut cfg = ForgeConfig::default();
        // Remove any stale env vars
        std::env::remove_var("FORGE_HTTP_ENABLED");
        std::env::remove_var("FORGE_HTTP_PORT");
        std::env::remove_var("FORGE_AUTH_ENABLED");

        cfg.apply_env_overrides();

        // Should remain defaults
        assert!(!cfg.http.enabled);
        assert_eq!(cfg.http.port, 8420);
        assert!(!cfg.auth.enabled);
    }

    #[test]
    #[serial]
    fn test_env_override_invalid_port_ignored() {
        let mut cfg = ForgeConfig::default();
        std::env::set_var("FORGE_HTTP_PORT", "not_a_number");

        cfg.apply_env_overrides();

        // Should remain at default since parse failed
        assert_eq!(cfg.http.port, 8420);

        std::env::remove_var("FORGE_HTTP_PORT");
    }

    #[test]
    fn test_reality_config_update_at() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let path_str = path.to_str().unwrap();
        std::fs::write(&path, "").unwrap();

        // Update all reality keys
        update_config_at(path_str, "reality.auto_detect", "false").unwrap();
        update_config_at(path_str, "reality.code_embeddings", "true").unwrap();
        update_config_at(path_str, "reality.community_detection", "false").unwrap();
        update_config_at(path_str, "reality.max_index_files", "10000").unwrap();

        let cfg = load_config_from(path_str);
        assert!(!cfg.reality.auto_detect);
        assert!(cfg.reality.code_embeddings);
        assert!(!cfg.reality.community_detection);
        assert_eq!(cfg.reality.max_index_files, 10000);

        // Invalid value should error
        let err = update_config_at(path_str, "reality.auto_detect", "not_a_bool");
        assert!(err.is_err());
    }

    // -----------------------------------------------------------------------
    // OTLP config tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_otlp_config_defaults() {
        let cfg = OtlpConfig::default();
        assert!(!cfg.enabled, "otlp.enabled default should be false");
        assert!(
            cfg.endpoint.is_empty(),
            "otlp.endpoint default should be empty"
        );
        assert_eq!(
            cfg.service_name, "forge-daemon",
            "otlp.service_name default should be forge-daemon"
        );

        // Also verify it shows up in ForgeConfig
        let forge_cfg = ForgeConfig::default();
        assert!(!forge_cfg.otlp.enabled);
        assert!(forge_cfg.otlp.endpoint.is_empty());
        assert_eq!(forge_cfg.otlp.service_name, "forge-daemon");
    }

    #[test]
    fn test_otlp_config_from_toml() {
        let toml_str = r#"
[otlp]
enabled = true
endpoint = "http://localhost:4317"
service_name = "my-forge"
"#;
        let cfg: ForgeConfig = toml::from_str(toml_str).unwrap();
        assert!(cfg.otlp.enabled);
        assert_eq!(cfg.otlp.endpoint, "http://localhost:4317");
        assert_eq!(cfg.otlp.service_name, "my-forge");
    }

    #[test]
    #[serial]
    fn test_otlp_env_override() {
        let mut cfg = ForgeConfig::default();
        std::env::set_var("FORGE_OTLP_ENABLED", "true");
        std::env::set_var("FORGE_OTLP_ENDPOINT", "http://jaeger:4317");
        std::env::set_var("FORGE_OTLP_SERVICE_NAME", "forge-prod");

        cfg.apply_env_overrides();

        assert!(cfg.otlp.enabled);
        assert_eq!(cfg.otlp.endpoint, "http://jaeger:4317");
        assert_eq!(cfg.otlp.service_name, "forge-prod");

        std::env::remove_var("FORGE_OTLP_ENABLED");
        std::env::remove_var("FORGE_OTLP_ENDPOINT");
        std::env::remove_var("FORGE_OTLP_SERVICE_NAME");
    }

    // -----------------------------------------------------------------------
    // Session reaper / heartbeat config tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_default_reaper_config() {
        let config = ForgeConfig::default();
        assert_eq!(
            config.workers.session_reaper_interval_secs, 60,
            "session_reaper_interval_secs default should be 60"
        );
    }

    #[test]
    fn test_default_heartbeat_timeout() {
        // Phase 2A-4d.3.1 #7: bumped from 60s (too aggressive — operators
        // report many "active" sessions because the previous binary
        // active→ended transition fired every 5 min) to 14400s (4h).
        // Idle phase at 600s gives intermediate visibility.
        let config = ForgeConfig::default();
        assert_eq!(
            config.workers.heartbeat_timeout_secs, 14_400,
            "heartbeat_timeout_secs default should be 14400 (4h)"
        );
        assert_eq!(
            config.workers.heartbeat_idle_secs, 600,
            "heartbeat_idle_secs default should be 600 (10 min)"
        );
    }

    #[test]
    fn test_validated_idle_clamps_below_timeout() {
        // Phase 2A-4d.3.1 #7: idle must be < timeout to be reachable.
        // If misconfigured to be greater, validated() collapses it.
        let mut cfg = WorkerConfig::default();
        cfg.heartbeat_timeout_secs = 100;
        cfg.heartbeat_idle_secs = 999;
        let v = cfg.validated();
        assert_eq!(v.heartbeat_timeout_secs, 100);
        assert!(v.heartbeat_idle_secs < v.heartbeat_timeout_secs);
        assert_eq!(v.heartbeat_idle_secs, 99);
    }

    #[test]
    fn test_validated_idle_zero_disables_phase() {
        let mut cfg = WorkerConfig::default();
        cfg.heartbeat_idle_secs = 0;
        let v = cfg.validated();
        assert_eq!(v.heartbeat_idle_secs, 0, "0 must stay 0 (disabled)");
    }

    #[test]
    #[serial]
    fn test_env_override_session_reaper() {
        let mut cfg = ForgeConfig::default();
        std::env::set_var("FORGE_SESSION_REAPER_INTERVAL", "120");
        std::env::set_var("FORGE_HEARTBEAT_TIMEOUT", "90");

        cfg.apply_env_overrides();

        assert_eq!(cfg.workers.session_reaper_interval_secs, 120);
        assert_eq!(cfg.workers.heartbeat_timeout_secs, 90);

        std::env::remove_var("FORGE_SESSION_REAPER_INTERVAL");
        std::env::remove_var("FORGE_HEARTBEAT_TIMEOUT");
    }

    #[test]
    fn test_reaper_config_from_toml() {
        let toml_str = r#"
[workers]
session_reaper_interval_secs = 30
heartbeat_timeout_secs = 45
"#;
        let cfg: ForgeConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.workers.session_reaper_interval_secs, 30);
        assert_eq!(cfg.workers.heartbeat_timeout_secs, 45);
    }

    #[test]
    fn test_reaper_config_update_at() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let path_str = path.to_str().unwrap();
        std::fs::write(&path, "").unwrap();

        update_config_at(path_str, "workers.session_reaper_interval_secs", "120").unwrap();
        update_config_at(path_str, "workers.heartbeat_timeout_secs", "90").unwrap();
        let cfg = load_config_from(path_str);
        assert_eq!(cfg.workers.session_reaper_interval_secs, 120);
        assert_eq!(cfg.workers.heartbeat_timeout_secs, 90);
    }

    #[test]
    fn test_proactive_config_defaults() {
        let config = ForgeConfig::default();
        assert_eq!(config.proactive.refresh_budget_chars, 300);
        assert_eq!(config.proactive.completion_check_budget_chars, 200);
        assert_eq!(config.proactive.subagent_context_budget_chars, 500);
        assert!((config.proactive.anti_pattern_threshold - 0.85).abs() < 0.01);
        assert!(!config.proactive.completion_keywords.is_empty());
        assert_eq!(config.proactive.completion_dismiss_limit, 3);
    }

    #[test]
    fn recall_config_default_preference_half_life_days() {
        let cfg = RecallConfig::default();
        assert_eq!(cfg.preference_half_life_days, 14.0);
    }

    #[test]
    fn recall_config_validated_clamps_preference_half_life_days() {
        let cfg = RecallConfig {
            preference_half_life_days: 0.0,
            ..RecallConfig::default()
        };
        assert_eq!(cfg.validated().preference_half_life_days, 1.0);

        let cfg = RecallConfig {
            preference_half_life_days: 1000.0,
            ..RecallConfig::default()
        };
        assert_eq!(cfg.validated().preference_half_life_days, 365.0);

        let cfg = RecallConfig {
            preference_half_life_days: 30.0,
            ..RecallConfig::default()
        };
        assert_eq!(cfg.validated().preference_half_life_days, 30.0);

        // Exact boundary preservation
        let cfg = RecallConfig {
            preference_half_life_days: 1.0,
            ..RecallConfig::default()
        };
        assert_eq!(
            cfg.validated().preference_half_life_days,
            1.0,
            "lower boundary 1.0 must preserve"
        );

        let cfg = RecallConfig {
            preference_half_life_days: 365.0,
            ..RecallConfig::default()
        };
        assert_eq!(
            cfg.validated().preference_half_life_days,
            365.0,
            "upper boundary 365.0 must preserve"
        );
    }

    // ── Phase 2A-4c2 T2: skill inference config validator tests ──────────────

    #[test]
    fn consolidation_config_default_skill_inference_values() {
        let cfg = ConsolidationConfig::default();
        assert_eq!(cfg.skill_inference_min_sessions, 3);
        assert_eq!(cfg.skill_inference_window_days, 30);
        assert!((cfg.skill_inference_tool_name_similarity_threshold - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn consolidation_config_validated_clamps_min_sessions_in_range() {
        let mut cfg = ConsolidationConfig::default();
        cfg.skill_inference_min_sessions = 0;
        assert_eq!(
            cfg.validated().skill_inference_min_sessions,
            1,
            "0 clamps to range min 1"
        );

        cfg.skill_inference_min_sessions = 50;
        assert_eq!(
            cfg.validated().skill_inference_min_sessions,
            20,
            "50 clamps to range max 20"
        );

        cfg.skill_inference_min_sessions = 5;
        assert_eq!(
            cfg.validated().skill_inference_min_sessions,
            5,
            "in-range value preserved"
        );
    }

    #[test]
    fn consolidation_config_validated_clamps_window_days_in_range() {
        let mut cfg = ConsolidationConfig::default();
        cfg.skill_inference_window_days = 0;
        assert_eq!(cfg.validated().skill_inference_window_days, 1);
        cfg.skill_inference_window_days = 400;
        assert_eq!(cfg.validated().skill_inference_window_days, 365);
    }

    #[test]
    fn consolidation_config_validated_clamps_similarity_threshold() {
        let mut cfg = ConsolidationConfig::default();
        cfg.skill_inference_tool_name_similarity_threshold = -1.0;
        assert_eq!(
            cfg.validated()
                .skill_inference_tool_name_similarity_threshold,
            0.0
        );
        cfg.skill_inference_tool_name_similarity_threshold = 2.5;
        assert_eq!(
            cfg.validated()
                .skill_inference_tool_name_similarity_threshold,
            1.0
        );
    }
}
