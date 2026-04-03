use crate::client;

/// Print Manas 8-layer memory health.
pub async fn manas_health() {
    let request = serde_json::json!({
        "method": "manas_health",
        "params": {}
    });

    match client::send_raw(&request).await {
        Ok(resp) => {
            if resp.get("status").and_then(|s| s.as_str()) == Some("error") {
                let msg = resp
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("unknown error");
                eprintln!("error: {msg}");
                std::process::exit(1);
            }

            let data = match resp.get("data") {
                Some(d) => d,
                None => {
                    eprintln!("unexpected response: no data field");
                    std::process::exit(1);
                }
            };

            let layers = data.get("layers").and_then(|l| l.as_array());
            let ahankara = data.get("ahankara");
            let disposition = data.get("disposition");

            println!("Manas 8-Layer Memory Health");
            println!("\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}");

            if let Some(layers) = layers {
                for layer in layers {
                    let num = layer.get("layer").and_then(|n| n.as_u64()).unwrap_or(0);
                    let name = layer
                        .get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or("Unknown");
                    let count = layer.get("count").and_then(|c| c.as_u64()).unwrap_or(0);
                    let unit = layer
                        .get("unit")
                        .and_then(|u| u.as_str())
                        .unwrap_or("entries");
                    println!(
                        "Layer {num} ({name}): {count:>5} {unit}",
                    );
                }
            } else {
                // Fallback: try to extract individual layer counts from flat fields
                let platform = data.get("platform_count").and_then(|c| c.as_u64()).unwrap_or(0);
                let tools = data.get("tool_count").and_then(|c| c.as_u64()).unwrap_or(0);
                let skills = data.get("skill_count").and_then(|c| c.as_u64()).unwrap_or(0);
                let domain = data.get("domain_count").and_then(|c| c.as_u64()).unwrap_or(0);
                let experience = data.get("experience_count").and_then(|c| c.as_u64()).unwrap_or(0);
                let perception = data.get("perception_count").and_then(|c| c.as_u64()).unwrap_or(0);
                let declared = data.get("declared_count").and_then(|c| c.as_u64()).unwrap_or(0);
                let latent = data.get("latent_count").and_then(|c| c.as_u64()).unwrap_or(0);

                println!("Layer 1 (Platform):     {platform:>3} entries");
                println!("Layer 2 (Tool):         {tools:>3} tools");
                println!("Layer 3 (Skill):        {skills:>3} skills");
                println!("Layer 4 (Domain DNA):   {domain:>3} patterns");
                println!("Layer 5 (Experience):   {experience:>3} memories");
                println!("Layer 6 (Perception):   {perception:>3} unconsumed");
                println!("Layer 7 (Declared):     {declared:>3} documents");
                println!("Layer 8 (Latent):       {latent:>3} embeddings");
            }

            println!("\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}");

            let facet_count = ahankara
                .and_then(|a| a.get("facet_count"))
                .and_then(|c| c.as_u64())
                .unwrap_or(0);
            let trait_count = disposition
                .and_then(|d| d.get("trait_count"))
                .and_then(|c| c.as_u64())
                .unwrap_or(0);

            println!("Ahankara (Identity):    {facet_count} facets");
            println!("Disposition:            {trait_count} traits");
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

/// List identity facets for an agent.
pub async fn identity_list(agent: String) {
    let request = serde_json::json!({
        "method": "identity_list",
        "params": {
            "agent": agent
        }
    });

    match client::send_raw(&request).await {
        Ok(resp) => {
            if resp.get("status").and_then(|s| s.as_str()) == Some("error") {
                let msg = resp
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("unknown error");
                eprintln!("error: {msg}");
                std::process::exit(1);
            }

            let data = match resp.get("data") {
                Some(d) => d,
                None => {
                    eprintln!("unexpected response: no data field");
                    std::process::exit(1);
                }
            };

            let facets = data.get("facets").and_then(|f| f.as_array());

            println!("Identity Facets ({agent})");
            println!("\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}");

            match facets {
                Some(facets) if !facets.is_empty() => {
                    for facet in facets {
                        let strength = facet
                            .get("strength")
                            .and_then(|s| s.as_f64())
                            .unwrap_or(0.0);
                        let facet_type = facet
                            .get("facet_type")
                            .and_then(|t| t.as_str())
                            .unwrap_or("unknown");
                        let description = facet
                            .get("description")
                            .and_then(|d| d.as_str())
                            .unwrap_or("");
                        let source = facet
                            .get("source")
                            .and_then(|s| s.as_str())
                            .unwrap_or("unknown");
                        println!("[{strength:.2}] {facet_type}: {description} (source: {source})");
                    }
                }
                _ => {
                    println!("  (no identity facets defined)");
                }
            }
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

/// Set an identity facet.
pub async fn identity_set(
    facet: String,
    description: String,
    agent: String,
    strength: f64,
) {
    let request = serde_json::json!({
        "method": "identity_set",
        "params": {
            "agent": agent,
            "facet_type": facet,
            "description": description,
            "strength": strength
        }
    });

    match client::send_raw(&request).await {
        Ok(resp) => {
            if resp.get("status").and_then(|s| s.as_str()) == Some("error") {
                let msg = resp
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("unknown error");
                eprintln!("error: {msg}");
                std::process::exit(1);
            }

            let data = resp.get("data");
            let id = data
                .and_then(|d| d.get("id"))
                .and_then(|i| i.as_str())
                .unwrap_or("(unknown)");
            println!("Identity facet set: {id}");
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

/// Remove (deactivate) an identity facet by ID.
pub async fn identity_remove(id: String) {
    let request = serde_json::json!({
        "method": "identity_remove",
        "params": {
            "id": id
        }
    });

    match client::send_raw(&request).await {
        Ok(resp) => {
            if resp.get("status").and_then(|s| s.as_str()) == Some("error") {
                let msg = resp
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("unknown error");
                eprintln!("error: {msg}");
                std::process::exit(1);
            }

            println!("Identity facet deactivated: {id}");
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

/// Show platform information (Layer 1).
pub async fn platform() {
    let request = serde_json::json!({
        "method": "platform_info",
        "params": {}
    });

    match client::send_raw(&request).await {
        Ok(resp) => {
            if resp.get("status").and_then(|s| s.as_str()) == Some("error") {
                let msg = resp
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("unknown error");
                eprintln!("error: {msg}");
                std::process::exit(1);
            }

            let data = match resp.get("data") {
                Some(d) => d,
                None => {
                    eprintln!("unexpected response: no data field");
                    std::process::exit(1);
                }
            };

            println!("Platform Information (Layer 1)");
            println!("\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}");

            let entries = data.get("entries").and_then(|e| e.as_array());
            match entries {
                Some(entries) if !entries.is_empty() => {
                    for entry in entries {
                        let key = entry
                            .get("key")
                            .and_then(|k| k.as_str())
                            .unwrap_or("unknown");
                        let value = entry
                            .get("value")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        println!("  {key}: {value}");
                    }
                }
                _ => {
                    println!("  (no platform information available)");
                }
            }
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

/// List discovered tools (Layer 2).
pub async fn tools() {
    let request = serde_json::json!({
        "method": "tools_list",
        "params": {}
    });

    match client::send_raw(&request).await {
        Ok(resp) => {
            if resp.get("status").and_then(|s| s.as_str()) == Some("error") {
                let msg = resp
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("unknown error");
                eprintln!("error: {msg}");
                std::process::exit(1);
            }

            let data = match resp.get("data") {
                Some(d) => d,
                None => {
                    eprintln!("unexpected response: no data field");
                    std::process::exit(1);
                }
            };

            println!("Discovered Tools (Layer 2)");
            println!("\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}");

            let tools = data.get("tools").and_then(|t| t.as_array());
            match tools {
                Some(tools) if !tools.is_empty() => {
                    for tool in tools {
                        let name = tool
                            .get("name")
                            .and_then(|n| n.as_str())
                            .unwrap_or("unknown");
                        let provider = tool
                            .get("provider")
                            .and_then(|p| p.as_str())
                            .unwrap_or("");
                        let usage = tool
                            .get("usage_count")
                            .and_then(|u| u.as_u64())
                            .unwrap_or(0);
                        println!("  {name} (provider: {provider}, used: {usage}x)");
                    }
                }
                _ => {
                    println!("  (no tools discovered yet)");
                }
            }
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

/// List unconsumed perceptions (Layer 6).
pub async fn perceptions(project: Option<String>, limit: usize) {
    let request = serde_json::json!({
        "method": "perceptions_list",
        "params": {
            "project": project,
            "limit": limit
        }
    });

    match client::send_raw(&request).await {
        Ok(resp) => {
            if resp.get("status").and_then(|s| s.as_str()) == Some("error") {
                let msg = resp
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("unknown error");
                eprintln!("error: {msg}");
                std::process::exit(1);
            }

            let data = match resp.get("data") {
                Some(d) => d,
                None => {
                    eprintln!("unexpected response: no data field");
                    std::process::exit(1);
                }
            };

            let perceptions = data.get("perceptions").and_then(|p| p.as_array());
            let count = data.get("count").and_then(|c| c.as_u64()).unwrap_or(0);

            let proj_label = project
                .as_deref()
                .unwrap_or("all projects");
            println!("Unconsumed Perceptions ({proj_label}) - {count} total");
            println!("\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}");

            match perceptions {
                Some(perceptions) if !perceptions.is_empty() => {
                    for (i, p) in perceptions.iter().enumerate() {
                        let content = p
                            .get("content")
                            .and_then(|c| c.as_str())
                            .unwrap_or("");
                        let source = p
                            .get("source")
                            .and_then(|s| s.as_str())
                            .unwrap_or("unknown");
                        let ts = p
                            .get("created_at")
                            .and_then(|t| t.as_str())
                            .unwrap_or("");
                        println!("  [{}] {} (source: {}, at: {})", i + 1, content, source, ts);
                    }
                }
                _ => {
                    println!("  (no unconsumed perceptions)");
                }
            }
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}
