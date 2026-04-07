// workspace.rs — Physical workspace management for organizations, teams, and agents.
//
// Provides directory layout, artifact writing (decisions, meeting minutes),
// and backlog reading for team workspaces.

use std::io;
use std::path::{Path, PathBuf};

use crate::config::WorkspaceConfig;

/// Slugify a title: lowercase, replace spaces/non-alphanumeric with hyphens,
/// collapse consecutive hyphens, trim leading/trailing hyphens.
fn slugify(title: &str) -> String {
    let mut slug = String::with_capacity(title.len());
    for ch in title.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
        } else {
            // Replace any non-alphanumeric with hyphen
            if !slug.ends_with('-') {
                slug.push('-');
            }
        }
    }
    // Trim leading/trailing hyphens
    slug.trim_matches('-').to_string()
}

/// Get today's date as YYYY-MM-DD string (no external chrono dependency).
fn today_ymd() -> String {
    let ts = forge_core::time::now_iso(); // "YYYY-MM-DD HH:MM:SS"
    ts[..10].to_string()
}

/// Resolve the workspace root path for a team based on the workspace mode.
///
/// - `project`: returns `None` (no team workspaces in project mode)
/// - `team`: returns `{project_dir}/.forge/teams/{team_name}/`
/// - `distributed`: looks up `config.roots[team_name]`
/// - `centralized`: returns `{config.root}/orgs/{org_name}/teams/{team_name}/`
pub fn team_workspace_path(
    config: &WorkspaceConfig,
    team_name: &str,
    org_name: &str,
    project_dir: Option<&str>,
) -> Option<PathBuf> {
    match config.mode.as_str() {
        "project" => None,
        "team" => {
            let dir = project_dir?;
            Some(PathBuf::from(dir).join(".forge").join("teams").join(team_name))
        }
        "distributed" => {
            config.roots.get(team_name).map(PathBuf::from)
        }
        "centralized" => {
            if config.root.is_empty() {
                return None;
            }
            Some(
                PathBuf::from(&config.root)
                    .join("orgs")
                    .join(org_name)
                    .join("teams")
                    .join(team_name),
            )
        }
        _ => None,
    }
}

/// Initialize workspace directories for an organization.
/// Creates the full directory tree: teams/{name}/decisions/, meetings/, agents/, _shared/.
/// Writes org.json and workspace.json.
///
/// Returns the workspace root path.
pub fn init_org_workspace(
    config: &WorkspaceConfig,
    org_name: &str,
    team_names: &[String],
    project_dir: Option<&str>,
) -> io::Result<PathBuf> {
    // Determine workspace root based on mode
    let ws_root = match config.mode.as_str() {
        "team" => {
            let dir = project_dir.ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "team mode requires project_dir")
            })?;
            PathBuf::from(dir).join(".forge")
        }
        "centralized" => {
            if config.root.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "centralized mode requires workspace.root to be set",
                ));
            }
            PathBuf::from(&config.root).join("orgs").join(org_name)
        }
        "distributed" => {
            // In distributed mode, each team has its own root.
            // We create a virtual root under project_dir for org.json/workspace.json.
            let dir = project_dir.ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "distributed mode requires project_dir")
            })?;
            PathBuf::from(dir).join(".forge")
        }
        _ => {
            // project mode — still create .forge/ with workspace.json
            let dir = project_dir.ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "project mode requires project_dir")
            })?;
            PathBuf::from(dir).join(".forge")
        }
    };

    // Create workspace root
    std::fs::create_dir_all(&ws_root)?;

    // Create team directories (except in project mode with no teams)
    if config.mode != "project" {
        let teams_dir = ws_root.join("teams");
        std::fs::create_dir_all(&teams_dir)?;

        for team in team_names {
            let team_dir = teams_dir.join(team);
            std::fs::create_dir_all(team_dir.join("decisions"))?;
            std::fs::create_dir_all(team_dir.join("meetings"))?;
            std::fs::create_dir_all(team_dir.join("agents"))?;
        }

        // Create _shared directory
        std::fs::create_dir_all(teams_dir.join("_shared"))?;
    }

    // Write org.json
    let org_json = serde_json::json!({
        "name": org_name,
        "created_at": forge_core::time::now_iso(),
        "teams": team_names,
    });
    std::fs::write(
        ws_root.join("org.json"),
        serde_json::to_string_pretty(&org_json).unwrap_or_else(|_| "{}".to_string()),
    )?;

    // Write workspace.json
    let ws_json = serde_json::json!({
        "mode": config.mode,
        "org": org_name,
        "version": "1.0",
    });
    std::fs::write(
        ws_root.join("workspace.json"),
        serde_json::to_string_pretty(&ws_json).unwrap_or_else(|_| "{}".to_string()),
    )?;

    Ok(ws_root)
}

/// Write a decision to the team's decisions/ directory as Markdown.
///
/// Returns the path of the written file.
pub fn write_decision(
    workspace_root: &Path,
    team: &str,
    title: &str,
    content: &str,
    confidence: f64,
    tags: &[String],
    memory_id: &str,
) -> io::Result<PathBuf> {
    let date = today_ymd();
    let slug = slugify(title);
    let team_slug = slugify(team);
    let filename = format!("{}-{}.md", date, slug);
    let dir = workspace_root.join("teams").join(&team_slug).join("decisions");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(&filename);

    let tags_str = tags.join(", ");
    let md = format!(
        "# {title}\n\
         \n\
         > **Confidence:** {confidence:.2}\n\
         > **Date:** {date}\n\
         > **Team:** {team}\n\
         > **Tags:** {tags_str}\n\
         \n\
         {content}\n\
         \n\
         ## References\n\
         - Memory ID: {memory_id}\n",
    );

    std::fs::write(&path, md)?;
    Ok(path)
}

/// Write meeting minutes to the team's meetings/ directory as Markdown.
///
/// Returns the path of the written file.
pub fn write_meeting_minutes(
    workspace_root: &Path,
    team: &str,
    topic: &str,
    participants: &[String],
    contributions: &[(String, String)],
    decision: &str,
    meeting_id: &str,
) -> io::Result<PathBuf> {
    let date = today_ymd();
    let slug = slugify(topic);
    let filename = format!("{}-{}.md", date, slug);
    let dir = workspace_root.join("teams").join(team).join("meetings");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(&filename);

    let participants_str = participants.join(", ");

    let mut md = format!(
        "# {topic}\n\
         \n\
         > **Date:** {date}\n\
         > **Team:** {team}\n\
         > **Participants:** {participants_str}\n\
         > **Status:** Decided\n\
         \n\
         ## Topic\n\
         {topic}\n\
         \n\
         ## Contributions\n\
         \n",
    );

    for (participant, contribution) in contributions {
        md.push_str(&format!("### {}\n{}\n\n", participant, contribution));
    }

    md.push_str(&format!(
        "## Decision\n\
         {decision}\n\
         \n\
         > **Meeting ID:** {meeting_id}\n",
    ));

    std::fs::write(&path, md)?;
    Ok(path)
}

/// Read team backlog from `teams/{team}/backlog.md` if it exists.
pub fn read_team_backlog(workspace_root: &Path, team: &str) -> Option<String> {
    let path = workspace_root.join("teams").join(team).join("backlog.md");
    std::fs::read_to_string(path).ok()
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn make_config(mode: &str) -> WorkspaceConfig {
        WorkspaceConfig {
            mode: mode.to_string(),
            root: String::new(),
            org: String::new(),
            auto_write: crate::config::AutoWriteConfig::default(),
            roots: HashMap::new(),
        }
    }

    #[test]
    fn test_team_workspace_path_project_mode() {
        let config = make_config("project");
        let result = team_workspace_path(&config, "engineering", "MyOrg", Some("/home/user/proj"));
        assert!(result.is_none(), "project mode should return None");
    }

    #[test]
    fn test_team_workspace_path_team_mode() {
        let config = make_config("team");
        let result = team_workspace_path(&config, "engineering", "MyOrg", Some("/home/user/proj"));
        assert_eq!(
            result.unwrap(),
            PathBuf::from("/home/user/proj/.forge/teams/engineering")
        );
    }

    #[test]
    fn test_team_workspace_path_distributed_mode() {
        let mut config = make_config("distributed");
        config.roots.insert(
            "engineering".to_string(),
            "/data/eng-workspace".to_string(),
        );
        let result = team_workspace_path(&config, "engineering", "MyOrg", None);
        assert_eq!(result.unwrap(), PathBuf::from("/data/eng-workspace"));

        // Unknown team returns None
        let result2 = team_workspace_path(&config, "marketing", "MyOrg", None);
        assert!(result2.is_none(), "unknown team in distributed mode should return None");
    }

    #[test]
    fn test_team_workspace_path_centralized_mode() {
        let mut config = make_config("centralized");
        config.root = "/data/forge".to_string();
        let result = team_workspace_path(&config, "engineering", "AcmeCorp", None);
        assert_eq!(
            result.unwrap(),
            PathBuf::from("/data/forge/orgs/AcmeCorp/teams/engineering")
        );
    }

    #[test]
    fn test_team_workspace_path_centralized_no_root() {
        let config = make_config("centralized");
        let result = team_workspace_path(&config, "engineering", "AcmeCorp", None);
        assert!(result.is_none(), "centralized mode with empty root should return None");
    }

    #[test]
    fn test_init_org_workspace_creates_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let project_dir = tmp.path().to_str().unwrap();

        let config = make_config("team");
        let teams = vec!["engineering".to_string(), "marketing".to_string()];

        let ws_root = init_org_workspace(&config, "TestOrg", &teams, Some(project_dir)).unwrap();

        // Check workspace root is .forge
        assert_eq!(ws_root, PathBuf::from(project_dir).join(".forge"));

        // Check team directories
        assert!(ws_root.join("teams/engineering/decisions").is_dir());
        assert!(ws_root.join("teams/engineering/meetings").is_dir());
        assert!(ws_root.join("teams/engineering/agents").is_dir());
        assert!(ws_root.join("teams/marketing/decisions").is_dir());
        assert!(ws_root.join("teams/marketing/meetings").is_dir());
        assert!(ws_root.join("teams/marketing/agents").is_dir());

        // Check _shared directory
        assert!(ws_root.join("teams/_shared").is_dir());

        // Check org.json
        let org_json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(ws_root.join("org.json")).unwrap())
                .unwrap();
        assert_eq!(org_json["name"], "TestOrg");
        assert_eq!(org_json["teams"][0], "engineering");
        assert_eq!(org_json["teams"][1], "marketing");

        // Check workspace.json
        let ws_json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(ws_root.join("workspace.json")).unwrap())
                .unwrap();
        assert_eq!(ws_json["mode"], "team");
        assert_eq!(ws_json["org"], "TestOrg");
    }

    #[test]
    fn test_write_decision_markdown() {
        let tmp = tempfile::tempdir().unwrap();
        let ws_root = tmp.path();

        // Create the team decisions dir
        std::fs::create_dir_all(ws_root.join("teams/engineering/decisions")).unwrap();

        let tags = vec!["auth".to_string(), "security".to_string()];
        let path = write_decision(
            ws_root,
            "engineering",
            "Use JWT for Authentication",
            "We decided to use JWT tokens for HTTP API authentication.",
            0.95,
            &tags,
            "01KNJF6MT6K7GF4FK8T2BNTFND",
        )
        .unwrap();

        // Check filename
        let filename = path.file_name().unwrap().to_str().unwrap();
        assert!(filename.ends_with("-use-jwt-for-authentication.md"), "filename: {}", filename);
        assert!(filename.starts_with(&today_ymd()), "filename should start with today's date");

        // Check content
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("# Use JWT for Authentication"));
        assert!(content.contains("**Confidence:** 0.95"));
        assert!(content.contains("**Team:** engineering"));
        assert!(content.contains("**Tags:** auth, security"));
        assert!(content.contains("We decided to use JWT tokens"));
        assert!(content.contains("Memory ID: 01KNJF6MT6K7GF4FK8T2BNTFND"));
    }

    #[test]
    fn test_write_meeting_minutes_markdown() {
        let tmp = tempfile::tempdir().unwrap();
        let ws_root = tmp.path();

        // Create the team meetings dir
        std::fs::create_dir_all(ws_root.join("teams/c-suite/meetings")).unwrap();

        let participants = vec!["CTO".to_string(), "CFO".to_string(), "CMO".to_string()];
        let contributions = vec![
            ("CTO".to_string(), "Recommends per-seat pricing.".to_string()),
            ("CFO".to_string(), "Usage-based has higher ceiling.".to_string()),
        ];

        let path = write_meeting_minutes(
            ws_root,
            "c-suite",
            "Licensing Model",
            &participants,
            &contributions,
            "Per-seat pricing with 3 tiers.",
            "meeting-001",
        )
        .unwrap();

        // Check filename
        let filename = path.file_name().unwrap().to_str().unwrap();
        assert!(filename.ends_with("-licensing-model.md"), "filename: {}", filename);

        // Check content
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("# Licensing Model"));
        assert!(content.contains("**Participants:** CTO, CFO, CMO"));
        assert!(content.contains("**Status:** Decided"));
        assert!(content.contains("### CTO"));
        assert!(content.contains("Recommends per-seat pricing."));
        assert!(content.contains("### CFO"));
        assert!(content.contains("## Decision"));
        assert!(content.contains("Per-seat pricing with 3 tiers."));
        assert!(content.contains("**Meeting ID:** meeting-001"));
    }

    #[test]
    fn test_read_team_backlog() {
        let tmp = tempfile::tempdir().unwrap();
        let ws_root = tmp.path();

        // No backlog yet
        assert!(read_team_backlog(ws_root, "engineering").is_none());

        // Create backlog
        let team_dir = ws_root.join("teams/engineering");
        std::fs::create_dir_all(&team_dir).unwrap();
        std::fs::write(team_dir.join("backlog.md"), "# Backlog\n- Item 1\n- Item 2\n").unwrap();

        let backlog = read_team_backlog(ws_root, "engineering").unwrap();
        assert!(backlog.contains("# Backlog"));
        assert!(backlog.contains("- Item 1"));
    }

    #[test]
    fn test_slugify() {
        assert_eq!(slugify("Use JWT for Authentication"), "use-jwt-for-authentication");
        assert_eq!(slugify("Hello, World!"), "hello-world");
        assert_eq!(slugify("  spaces  "), "spaces");
        assert_eq!(slugify("CamelCase123"), "camelcase123");
        assert_eq!(slugify("a--b"), "a-b");
    }

    #[test]
    fn test_init_org_workspace_centralized_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_str().unwrap();

        let mut config = make_config("centralized");
        config.root = root.to_string();
        let teams = vec!["backend".to_string()];

        let ws_root = init_org_workspace(&config, "AcmeCorp", &teams, None).unwrap();

        assert_eq!(ws_root, PathBuf::from(root).join("orgs/AcmeCorp"));
        assert!(ws_root.join("teams/backend/decisions").is_dir());
        assert!(ws_root.join("teams/backend/meetings").is_dir());
        assert!(ws_root.join("teams/backend/agents").is_dir());
        assert!(ws_root.join("teams/_shared").is_dir());
        assert!(ws_root.join("org.json").exists());
        assert!(ws_root.join("workspace.json").exists());
    }

    #[test]
    fn test_write_decision_slugifies_team_name() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        // Path traversal attempt: team name like "../../etc/passwd"
        let result = write_decision(
            root,
            "../../etc/passwd",
            "Malicious Decision",
            "Content here",
            0.9,
            &["test".to_string()],
            "mem-123",
        );

        assert!(result.is_ok());
        let path = result.unwrap();
        // The path should NOT contain ".." — it should be slugified
        let path_str = path.to_string_lossy();
        assert!(!path_str.contains(".."), "slugified team path must not contain '..': {path_str}");
        // The slugified team name should be something like "etc-passwd"
        assert!(path_str.contains("etc-passwd"), "team name should be slugified to 'etc-passwd': {path_str}");
        // File should actually exist
        assert!(path.exists(), "decision file should have been written");
    }

    #[test]
    fn test_init_org_workspace_project_mode_no_team_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let project_dir = tmp.path().to_str().unwrap();

        let config = make_config("project");
        let teams = vec!["engineering".to_string()];

        let ws_root = init_org_workspace(&config, "Default", &teams, Some(project_dir)).unwrap();

        // In project mode, teams/ directory should NOT be created
        assert!(!ws_root.join("teams").exists());
        // But org.json and workspace.json should exist
        assert!(ws_root.join("org.json").exists());
        assert!(ws_root.join("workspace.json").exists());
    }
}
