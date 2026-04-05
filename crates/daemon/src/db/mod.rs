pub mod diagnostics;
pub mod manas;
pub mod schema;
pub mod ops;
pub mod vec;

pub use schema::create_schema;
pub use ops::{remember, recall_bm25, recall_bm25_project, forget, health, health_by_project, touch, count_files, count_symbols, store_edge, semantic_dedup, link_related_memories, embedding_merge, strengthen_active_edges, detect_contradictions, BM25Result, HealthCounts};
pub use ops::{create_default_org, get_organization, list_organizations, create_default_user, get_user, list_users, create_team, get_team, list_teams, add_team_member, list_team_members, store_reality, get_reality, get_reality_by_path, list_realities, update_reality_last_active, ensure_defaults};
