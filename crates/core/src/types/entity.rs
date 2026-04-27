use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Organization {
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForgeUser {
    pub id: String,
    pub name: String,
    pub email: Option<String>,
    pub organization_id: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Team {
    pub id: String,
    pub name: String,
    pub organization_id: String,
    pub created_by: String,
    pub status: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamMember {
    pub team_id: String,
    pub user_id: String,
    pub role: String,
    pub joined_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub reality_type: String,
    pub detected_from: Option<String>,
    pub project_path: Option<String>,
    pub domain: Option<String>,
    pub organization_id: String,
    pub owner_type: String,
    pub owner_id: String,
    pub engine_status: String,
    pub engine_pid: Option<i64>,
    pub created_at: String,
    pub last_active: String,
    pub metadata: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConfigScopeEntry {
    pub id: String,
    pub scope_type: String,
    pub scope_id: String,
    pub key: String,
    pub value: String,
    pub locked: bool,
    pub ceiling: Option<f64>,
    pub set_by: String,
    pub set_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResolvedConfigValue {
    pub key: String,
    pub value: String,
    pub source_scope_type: String,
    pub source_scope_id: String,
    pub locked: bool,
    pub ceiling_applied: bool,
}

/// Memory portability classification
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Portability {
    Universal,
    DomainTransferable,
    RealityBound,
    Unknown,
}

impl Portability {
    pub fn as_str(&self) -> &str {
        match self {
            Portability::Universal => "universal",
            Portability::DomainTransferable => "domain_transferable",
            Portability::RealityBound => "reality_bound",
            Portability::Unknown => "unknown",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "universal" => Portability::Universal,
            "domain_transferable" => Portability::DomainTransferable,
            "reality_bound" => Portability::RealityBound,
            _ => Portability::Unknown,
        }
    }
}

/// Memory visibility across scopes
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Visibility {
    /// Visible everywhere
    Universal,
    /// Visible in scope + children
    Inherited,
    /// This scope only
    Local,
    /// Creating agent only
    Private,
}

impl Visibility {
    pub fn as_str(&self) -> &str {
        match self {
            Visibility::Universal => "universal",
            Visibility::Inherited => "inherited",
            Visibility::Local => "local",
            Visibility::Private => "private",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "universal" => Visibility::Universal,
            "inherited" => Visibility::Inherited,
            "local" => Visibility::Local,
            "private" => Visibility::Private,
            _ => Visibility::Private,
        }
    }
}
