//! Role-Based Access Control (RBAC) for the HTTP API.
//!
//! Three roles with a simple permission matrix:
//! - **Admin**: all operations
//! - **Member**: read + write, but not admin operations (Shutdown, config, cleanup)
//! - **Viewer**: read-only operations only
//!
//! RBAC is only enforced on the HTTP path when auth is enabled.
//! Unix socket connections bypass RBAC completely (local = trusted).

use crate::server::auth::AuthClaims;
use forge_core::protocol::Request;

/// Role assigned to an authenticated user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Role {
    Admin,
    Member,
    Viewer,
}

impl Role {
    /// Returns the string representation of the role (used in audit logs).
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::Admin => "admin",
            Role::Member => "member",
            Role::Viewer => "viewer",
        }
    }
}

/// Resolve the role from JWT claims and the admin email list.
///
/// - If the user's email is in `admin_emails`, they get Admin.
/// - Otherwise, authenticated users default to Member.
pub fn resolve_role(claims: &AuthClaims, admin_emails: &[String]) -> Role {
    if let Some(ref email) = claims.email {
        if admin_emails.iter().any(|e| e == email) {
            return Role::Admin;
        }
    }
    Role::Member
}

/// Returns true if the request is an admin-only operation.
/// Uses an explicit list — new Request variants default to DENIED for Members
/// until explicitly classified (fail-closed).
fn is_admin_only(request: &Request) -> bool {
    matches!(
        request,
        Request::Shutdown
            | Request::SetConfig { .. }
            | Request::SetScopedConfig { .. }
            | Request::DeleteScopedConfig { .. }
            | Request::CleanupSessions { .. }
            | Request::GrantPermission { .. }
            | Request::RevokePermission { .. }
            | Request::Import { .. }
            | Request::SyncImport { .. }
            | Request::ForceIndex
    )
}

/// Check if the given role is allowed to perform the request.
///
/// Returns `Ok(())` if allowed, `Err(reason)` if denied.
pub fn check_permission(role: &Role, request: &Request) -> Result<(), String> {
    match role {
        Role::Admin => Ok(()), // Admin can do everything
        Role::Viewer => {
            // Viewers can only do read-only operations
            if crate::server::writer::is_read_only(request) {
                Ok(())
            } else {
                Err("insufficient permissions".to_string())
            }
        }
        Role::Member => {
            // Members can read and write, but not admin operations
            if is_admin_only(request) {
                Err("insufficient permissions".to_string())
            } else {
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_claims(email: Option<&str>) -> AuthClaims {
        AuthClaims {
            sub: "user-123".to_string(),
            email: email.map(|e| e.to_string()),
            groups: vec![],
            org: None,
            iss: None,
            aud: None,
            exp: None,
        }
    }

    // ── Role resolution tests ──

    #[test]
    fn test_resolve_role_admin_by_email() {
        let claims = make_claims(Some("admin@example.com"));
        let admin_emails = vec!["admin@example.com".to_string()];
        assert_eq!(resolve_role(&claims, &admin_emails), Role::Admin);
    }

    #[test]
    fn test_resolve_role_admin_multiple_emails() {
        let claims = make_claims(Some("boss@co.com"));
        let admin_emails = vec![
            "admin@example.com".to_string(),
            "boss@co.com".to_string(),
        ];
        assert_eq!(resolve_role(&claims, &admin_emails), Role::Admin);
    }

    #[test]
    fn test_resolve_role_member_when_not_in_admin_list() {
        let claims = make_claims(Some("user@example.com"));
        let admin_emails = vec!["admin@example.com".to_string()];
        assert_eq!(resolve_role(&claims, &admin_emails), Role::Member);
    }

    #[test]
    fn test_resolve_role_member_when_no_email() {
        let claims = make_claims(None);
        let admin_emails = vec!["admin@example.com".to_string()];
        assert_eq!(resolve_role(&claims, &admin_emails), Role::Member);
    }

    #[test]
    fn test_resolve_role_member_when_empty_admin_list() {
        let claims = make_claims(Some("admin@example.com"));
        let admin_emails: Vec<String> = vec![];
        assert_eq!(resolve_role(&claims, &admin_emails), Role::Member);
    }

    // ── Admin permission tests ──

    #[test]
    fn test_admin_can_do_everything() {
        assert!(check_permission(&Role::Admin, &Request::Health).is_ok());
        assert!(check_permission(&Role::Admin, &Request::Shutdown).is_ok());
        assert!(check_permission(
            &Role::Admin,
            &Request::Remember {
                memory_type: forge_core::types::MemoryType::Decision,
                title: "t".into(),
                content: "c".into(),
                confidence: None,
                tags: None,
                project: None,
            }
        )
        .is_ok());
        assert!(check_permission(
            &Role::Admin,
            &Request::SetConfig {
                key: "k".into(),
                value: "v".into(),
            }
        )
        .is_ok());
        assert!(check_permission(
            &Role::Admin,
            &Request::CleanupSessions { prefix: None }
        )
        .is_ok());
    }

    // ── Viewer permission tests ──

    #[test]
    fn test_viewer_can_read() {
        assert!(check_permission(&Role::Viewer, &Request::Health).is_ok());
        assert!(check_permission(&Role::Viewer, &Request::GetConfig).is_ok());
        assert!(check_permission(&Role::Viewer, &Request::Status).is_ok());
        assert!(check_permission(&Role::Viewer, &Request::Doctor).is_ok());
        assert!(check_permission(&Role::Viewer, &Request::LspStatus).is_ok());
        assert!(check_permission(
            &Role::Viewer,
            &Request::Recall {
                query: "test".into(),
                memory_type: None,
                project: None,
                limit: None,
                layer: None,
            }
        )
        .is_ok());
    }

    #[test]
    fn test_viewer_blocked_from_writes() {
        let result = check_permission(
            &Role::Viewer,
            &Request::Remember {
                memory_type: forge_core::types::MemoryType::Decision,
                title: "t".into(),
                content: "c".into(),
                confidence: None,
                tags: None,
                project: None,
            },
        );
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("insufficient permissions"));
    }

    #[test]
    fn test_viewer_blocked_from_admin_ops() {
        assert!(check_permission(&Role::Viewer, &Request::Shutdown).is_err());
        assert!(check_permission(
            &Role::Viewer,
            &Request::SetConfig {
                key: "k".into(),
                value: "v".into(),
            }
        )
        .is_err());
        assert!(check_permission(
            &Role::Viewer,
            &Request::CleanupSessions { prefix: None }
        )
        .is_err());
    }

    // ── Member permission tests ──

    #[test]
    fn test_member_can_read() {
        assert!(check_permission(&Role::Member, &Request::Health).is_ok());
        assert!(check_permission(&Role::Member, &Request::GetConfig).is_ok());
        assert!(check_permission(
            &Role::Member,
            &Request::Recall {
                query: "test".into(),
                memory_type: None,
                project: None,
                limit: None,
                layer: None,
            }
        )
        .is_ok());
    }

    #[test]
    fn test_member_can_write() {
        assert!(check_permission(
            &Role::Member,
            &Request::Remember {
                memory_type: forge_core::types::MemoryType::Decision,
                title: "t".into(),
                content: "c".into(),
                confidence: None,
                tags: None,
                project: None,
            }
        )
        .is_ok());
        assert!(check_permission(
            &Role::Member,
            &Request::Forget { id: "x".into() }
        )
        .is_ok());
        assert!(check_permission(
            &Role::Member,
            &Request::RegisterSession {
                id: "s".into(),
                agent: "a".into(),
                project: None,
                cwd: None,
                capabilities: None,
                current_task: None,
            }
        )
        .is_ok());
    }

    #[test]
    fn test_member_blocked_from_shutdown() {
        let result = check_permission(&Role::Member, &Request::Shutdown);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("insufficient permissions"));
    }

    #[test]
    fn test_member_blocked_from_set_config() {
        let result = check_permission(
            &Role::Member,
            &Request::SetConfig {
                key: "k".into(),
                value: "v".into(),
            },
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_member_blocked_from_set_scoped_config() {
        let result = check_permission(
            &Role::Member,
            &Request::SetScopedConfig {
                scope_type: "org".into(),
                scope_id: "default".into(),
                key: "k".into(),
                value: "v".into(),
                locked: false,
                ceiling: None,
            },
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_member_blocked_from_delete_scoped_config() {
        let result = check_permission(
            &Role::Member,
            &Request::DeleteScopedConfig {
                scope_type: "org".into(),
                scope_id: "default".into(),
                key: "k".into(),
            },
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_member_blocked_from_cleanup_sessions() {
        let result =
            check_permission(&Role::Member, &Request::CleanupSessions { prefix: None });
        assert!(result.is_err());
    }

    // ── Role::as_str tests ──

    #[test]
    fn test_role_as_str() {
        assert_eq!(Role::Admin.as_str(), "admin");
        assert_eq!(Role::Member.as_str(), "member");
        assert_eq!(Role::Viewer.as_str(), "viewer");
    }
}
