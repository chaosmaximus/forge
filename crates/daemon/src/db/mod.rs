pub mod diagnostics;
pub mod effectiveness;
pub mod manas;
pub mod ops;
pub mod raw;
pub mod schema;
pub mod vec;

pub use ops::classify_portability;
pub use ops::{
    add_team_member, create_default_org, create_default_user, create_team, ensure_defaults,
    get_organization, get_reality, get_reality_by_path, get_team, get_user, list_organizations,
    list_realities, list_team_members, list_teams, list_users, store_reality,
    update_reality_last_active,
};
pub use ops::{
    count_files, count_symbols, detect_contradictions, embedding_merge, forget, health,
    health_by_project, link_related_memories, recall_bm25, recall_bm25_project, remember,
    semantic_dedup, store_edge, strengthen_active_edges, touch, BM25Result, HealthCounts,
};
pub use ops::{
    delete_scoped_config, get_scoped_config, list_scoped_config, resolve_effective_config,
    resolve_scoped_config, set_scoped_config, validate_scope_type,
};
pub use schema::create_schema;
