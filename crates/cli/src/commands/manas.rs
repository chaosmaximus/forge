use crate::client;
use forge_core::protocol::{Request, Response, ResponseData};
use forge_core::types::manas::IdentityFacet;

/// Print Manas 8-layer memory health.
pub async fn manas_health() {
    let request = Request::ManasHealth { project: None };

    match client::send(&request).await {
        Ok(Response::Ok {
            data:
                ResponseData::ManasHealthData {
                    platform_count,
                    tool_count,
                    skill_count,
                    domain_dna_count,
                    perception_unconsumed,
                    declared_count,
                    identity_facets,
                    disposition_traits,
                    experience_count,
                    embedding_count,
                    trait_names,
                    is_new_project: _,
                },
        }) => {
            println!("Manas 8-Layer Memory Health");
            println!("\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}");
            println!("Layer 1 (Platform):     {platform_count:>3} entries");
            println!("Layer 2 (Tool):         {tool_count:>3} tools");
            println!("Layer 3 (Skill):        {skill_count:>3} skills");
            println!("Layer 4 (Domain DNA):   {domain_dna_count:>3} patterns");
            println!("Layer 5 (Experience):   {experience_count:>3} memories");
            println!("Layer 6 (Perception):   {perception_unconsumed:>3} unconsumed");
            println!("Layer 7 (Declared):     {declared_count:>3} documents");
            println!("Layer 8 (Latent):       {embedding_count:>3} embeddings");
            println!("\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}");
            println!("Ahankara (Identity):    {identity_facets} facets");
            if trait_names.is_empty() {
                println!("Disposition:            {disposition_traits} traits");
            } else {
                println!(
                    "Disposition:            {} traits ({})",
                    disposition_traits,
                    trait_names.join(", ")
                );
            }
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => {
            eprintln!("unexpected response");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

/// List identity facets for an agent.
pub async fn identity_list(agent: String) {
    let request = Request::ListIdentity {
        agent: agent.clone(),
    };

    match client::send(&request).await {
        Ok(Response::Ok {
            data: ResponseData::IdentityList { facets, count: _ },
        }) => {
            println!("Identity Facets ({agent})");
            println!("\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}");

            if facets.is_empty() {
                println!("  (no identity facets defined)");
            } else {
                for f in &facets {
                    println!(
                        "[{:.2}] {}: {} (source: {})",
                        f.strength, f.facet, f.description, f.source
                    );
                }
            }
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => {
            eprintln!("unexpected response");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

/// Set an identity facet.
pub async fn identity_set(facet: String, description: String, agent: String, strength: f64) {
    // Generate a timestamp-based ID
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let id = format!("idfacet-{}", now.as_millis());
    let created_at = format!("{}", now.as_secs());

    let identity_facet = IdentityFacet {
        id,
        agent,
        facet,
        description,
        strength,
        source: "cli".to_string(),
        active: true,
        created_at,
        user_id: None,
    };

    let request = Request::StoreIdentity {
        facet: identity_facet,
    };

    match client::send(&request).await {
        Ok(Response::Ok {
            data: ResponseData::IdentityStored { id },
        }) => {
            println!("Identity facet set: {id}");
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => {
            eprintln!("unexpected response");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

/// Remove (deactivate) an identity facet by ID.
pub async fn identity_remove(id: String) {
    let request = Request::DeactivateIdentity { id: id.clone() };

    match client::send(&request).await {
        Ok(Response::Ok {
            data:
                ResponseData::IdentityDeactivated {
                    id: deactivated_id,
                    found,
                },
        }) => {
            if found {
                println!("Identity facet deactivated: {deactivated_id}");
            } else {
                eprintln!("Identity facet not found: {deactivated_id}");
                std::process::exit(1);
            }
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => {
            eprintln!("unexpected response");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

/// Show platform information (Layer 1).
pub async fn platform() {
    let request = Request::ListPlatform;

    match client::send(&request).await {
        Ok(Response::Ok {
            data: ResponseData::PlatformList { entries },
        }) => {
            println!("Platform Information (Layer 1)");
            println!("\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}");

            if entries.is_empty() {
                println!("  (no platform information available)");
            } else {
                for entry in &entries {
                    println!("  {}: {}", entry.key, entry.value);
                }
            }
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => {
            eprintln!("unexpected response");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

/// List discovered tools (Layer 2).
pub async fn tools() {
    let request = Request::ListTools;

    match client::send(&request).await {
        Ok(Response::Ok {
            data: ResponseData::ToolList { tools, count: _ },
        }) => {
            println!("Discovered Tools (Layer 2)");
            println!("\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}");

            if tools.is_empty() {
                println!("  (no tools discovered yet)");
            } else {
                for tool in &tools {
                    println!(
                        "  {} (kind: {:?}, used: {}x)",
                        tool.name, tool.kind, tool.use_count
                    );
                }
            }
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => {
            eprintln!("unexpected response");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

/// List unconsumed perceptions (Layer 6).
pub async fn perceptions(project: Option<String>, limit: usize, offset: usize) {
    let request = Request::ListPerceptions {
        project: project.clone(),
        limit: Some(limit),
        offset: if offset > 0 { Some(offset) } else { None },
    };

    match client::send(&request).await {
        Ok(Response::Ok {
            data: ResponseData::PerceptionList { perceptions, count },
        }) => {
            let proj_label = project.as_deref().unwrap_or("all projects");
            println!("Unconsumed Perceptions ({proj_label}) - {count} total");
            println!("\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}");

            if perceptions.is_empty() {
                println!("  (no unconsumed perceptions)");
            } else {
                for (i, p) in perceptions.iter().enumerate() {
                    println!(
                        "  [{}] {} (kind: {:?}, severity: {:?}, at: {})",
                        i + 1,
                        p.data,
                        p.kind,
                        p.severity,
                        p.created_at
                    );
                }
            }
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => {
            eprintln!("unexpected response");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

/// Compile optimized context from all Manas layers (for session-start injection).
pub async fn compile_context(
    agent: String,
    project: Option<String>,
    static_only: bool,
    session_id: Option<String>,
    focus: Option<String>,
) {
    let request = Request::CompileContext {
        agent: Some(agent),
        project,
        static_only: if static_only { Some(true) } else { None },
        excluded_layers: None,
        session_id,
        focus,
    };

    match client::send(&request).await {
        Ok(Response::Ok {
            data:
                ResponseData::CompiledContext {
                    context,
                    layers_used,
                    chars,
                    ..
                },
        }) => {
            if context.is_empty() {
                println!("<forge-context version=\"0.7.0\"/>");
            } else {
                println!("{context}");
            }
            eprintln!("({layers_used} layers, {chars} chars)");
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => {
            eprintln!("unexpected response");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}
