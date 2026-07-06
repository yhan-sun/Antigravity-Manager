//! Build an Antigravity-like structured system instruction for Gemini.
//!
//! Codex sends several large system/developer blocks. Gemini handles this more
//! reliably when the stable parts are organized into one explicit document,
//! while volatile conversation state remains in `contents` and callable powers
//! remain in `tools`.

const MAX_CUSTOMIZATION_CHARS: usize = 12_000;
const MAX_PERMISSIONS_CHARS: usize = 4_000;
const MAX_APP_CONTEXT_CHARS: usize = 3_000;
const MAX_MEMORY_LINES: usize = 28;
const MAX_SKILLS: usize = 80;

#[derive(Debug, Clone)]
struct SkillIndexEntry {
    name: String,
    description: String,
}

#[derive(Debug, Default)]
struct ContextBlocks {
    identity: Option<String>,
    permissions: Vec<String>,
    app_context: Vec<String>,
    collaboration_mode: Vec<String>,
    skills: Vec<SkillIndexEntry>,
    plugins: Vec<String>,
    memory_lines: Vec<String>,
    customizations: Vec<String>,
    additional_instructions: Vec<String>,
}

/// Converts Codex/OpenAI system instructions into a single official-style
/// Gemini system instruction.
pub fn build_official_style_system_instruction(
    system_instructions: &[String],
    proxy_identity: Option<&str>,
    global_prompt: Option<&str>,
    request_type: &str,
    mapped_model: &str,
    session_id: &str,
) -> String {
    let mut blocks = ContextBlocks::default();

    if let Some(prompt) = global_prompt.map(str::trim).filter(|s| !s.is_empty()) {
        blocks.customizations.push(prompt.to_string());
    }

    let user_has_proxy_identity = system_instructions
        .iter()
        .any(|s| s.contains("You are Antigravity") || s.contains("You are a search engine bot"));
    let user_has_codex_identity = system_instructions
        .iter()
        .any(|s| s.contains("You are Codex"));

    for instruction in system_instructions {
        let mut handled_structured_block = false;

        if blocks.identity.is_none()
            && user_has_codex_identity
            && instruction.contains("You are Codex")
        {
            blocks.identity = Some(extract_identity_excerpt(instruction, "You are Codex"));
        } else if blocks.identity.is_none()
            && user_has_proxy_identity
            && instruction.contains("You are Antigravity")
        {
            blocks.identity = Some(extract_identity_excerpt(instruction, "You are Antigravity"));
        } else if blocks.identity.is_none()
            && user_has_proxy_identity
            && instruction.contains("You are a search engine bot")
        {
            blocks.identity = Some(extract_identity_excerpt(
                instruction,
                "You are a search engine bot",
            ));
        }

        for tag in [
            "permissions instructions",
            "app-context",
            "collaboration_mode",
            "skills_instructions",
            "plugins_instructions",
        ] {
            let extracted = extract_all_tag_blocks(instruction, tag);
            if extracted.is_empty() {
                continue;
            }
            handled_structured_block = true;

            match tag {
                "permissions instructions" => blocks.permissions.extend(extracted),
                "app-context" => blocks.app_context.extend(extracted),
                "collaboration_mode" => blocks.collaboration_mode.extend(extracted),
                "skills_instructions" => {
                    for block in &extracted {
                        blocks.skills.extend(extract_skills_index(block));
                    }
                }
                "plugins_instructions" => blocks.plugins.extend(extracted),
                _ => {}
            }
        }

        if instruction.contains("MEMORY_SUMMARY BEGINS") {
            blocks
                .memory_lines
                .extend(extract_relevant_memory_lines(instruction));
            handled_structured_block = true;
        }

        let trimmed_owned;
        let trimmed = if handled_structured_block {
            trimmed_owned = strip_known_structured_blocks(instruction);
            trimmed_owned.trim()
        } else {
            instruction.trim()
        };
        if should_keep_as_additional_instruction(trimmed) {
            blocks
                .additional_instructions
                .push(shorten(trimmed, MAX_CUSTOMIZATION_CHARS));
        }
    }

    if blocks.identity.is_none() {
        blocks.identity = proxy_identity
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned);
    }

    blocks.skills = dedupe_skills(blocks.skills);
    blocks.memory_lines = dedupe_lines(blocks.memory_lines);

    render_context_blocks(blocks, request_type, mapped_model, session_id)
}

fn render_context_blocks(
    blocks: ContextBlocks,
    request_type: &str,
    mapped_model: &str,
    session_id: &str,
) -> String {
    let mut out = String::new();

    append_section(
        &mut out,
        "identity",
        blocks
            .identity
            .as_deref()
            .unwrap_or("You are Antigravity, a powerful agentic AI coding assistant."),
    );

    append_section(
        &mut out,
        "user_information",
        &format!(
            "Request type: {}\nMapped model: {}\nSession ID: {}",
            request_type, mapped_model, session_id
        ),
    );

    if !blocks.permissions.is_empty() {
        append_section(
            &mut out,
            "environment_permissions",
            &summarize_permissions(&blocks.permissions.join("\n\n")),
        );
    }

    if !blocks.app_context.is_empty() {
        append_section(
            &mut out,
            "app_context",
            &shorten(&blocks.app_context.join("\n\n"), MAX_APP_CONTEXT_CHARS),
        );
    }

    if !blocks.customizations.is_empty() || !blocks.additional_instructions.is_empty() {
        let mut custom = Vec::new();
        custom.extend(blocks.customizations);
        custom.extend(blocks.additional_instructions);
        append_section(
            &mut out,
            "customizations",
            &shorten(&custom.join("\n\n"), MAX_CUSTOMIZATION_CHARS),
        );
    }

    if !blocks.skills.is_empty() {
        let mut skill_lines = vec![
            "Use specialized skills when they are relevant. The list below is an index; detailed behavior comes from the active tool/runtime when available.".to_string(),
        ];
        for skill in blocks.skills.iter().take(MAX_SKILLS) {
            skill_lines.push(format!("- {}: {}", skill.name, skill.description));
        }
        append_section(&mut out, "skills", &skill_lines.join("\n"));
    }

    if !blocks.plugins.is_empty() {
        append_section(
            &mut out,
            "plugins",
            &summarize_plugins(&blocks.plugins.join("\n\n")),
        );
    }

    if !blocks.memory_lines.is_empty() {
        append_section(
            &mut out,
            "memory",
            &blocks
                .memory_lines
                .iter()
                .take(MAX_MEMORY_LINES)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n"),
        );
    }

    if !blocks.collaboration_mode.is_empty() {
        append_section(
            &mut out,
            "planning_mode",
            &summarize_collaboration_mode(&blocks.collaboration_mode.join("\n\n")),
        );
    }

    append_section(
        &mut out,
        "communication_style",
        "Answer in the user's language. Keep status updates concise. Preserve tool calls and file edits as structured tool usage rather than describing them as plain text.",
    );

    out
}

fn append_section(out: &mut String, tag: &str, content: &str) {
    if content.trim().is_empty() {
        return;
    }
    if !out.is_empty() {
        out.push('\n');
    }
    out.push('<');
    out.push_str(tag);
    out.push_str(">\n");
    out.push_str(content.trim());
    out.push_str("\n</");
    out.push_str(tag);
    out.push('>');
}

fn strip_known_structured_blocks(text: &str) -> String {
    let mut cleaned = text.to_string();
    for tag in [
        "permissions instructions",
        "app-context",
        "collaboration_mode",
        "skills_instructions",
        "plugins_instructions",
    ] {
        cleaned = strip_all_tag_blocks(&cleaned, tag);
    }
    if let Some(start) = cleaned.find("========= MEMORY_SUMMARY BEGINS =========") {
        if let Some(end_rel) = cleaned[start..].find("========= MEMORY_SUMMARY ENDS =========") {
            let end = start + end_rel + "========= MEMORY_SUMMARY ENDS =========".len();
            cleaned.replace_range(start..end, "");
        }
    }
    collapse_blank_lines(&cleaned)
}

fn strip_all_tag_blocks(text: &str, tag: &str) -> String {
    let start_tag = format!("<{}>", tag);
    let end_tag = format!("</{}>", tag);
    let mut output = String::new();
    let mut rest = text;

    while let Some(start) = rest.find(&start_tag) {
        output.push_str(&rest[..start]);
        let after_start = &rest[start + start_tag.len()..];
        let Some(end) = after_start.find(&end_tag) else {
            output.push_str(after_start);
            return output;
        };
        rest = &after_start[end + end_tag.len()..];
    }
    output.push_str(rest);
    output
}

fn extract_all_tag_blocks(text: &str, tag: &str) -> Vec<String> {
    let start_tag = format!("<{}>", tag);
    let end_tag = format!("</{}>", tag);
    let mut result = Vec::new();
    let mut rest = text;

    while let Some(start) = rest.find(&start_tag) {
        let after_start = &rest[start + start_tag.len()..];
        let Some(end) = after_start.find(&end_tag) else {
            break;
        };
        result.push(after_start[..end].trim().to_string());
        rest = &after_start[end + end_tag.len()..];
    }

    result
}

fn extract_identity_excerpt(text: &str, marker: &str) -> String {
    let start = text.find(marker).unwrap_or(0);
    let excerpt = &text[start..];
    let end = excerpt
        .find("\n\n")
        .or_else(|| excerpt.find("<permissions instructions>"))
        .or_else(|| excerpt.find("<app-context>"))
        .unwrap_or_else(|| excerpt.len().min(2_000));
    shorten(&excerpt[..end], 2_000)
}

fn extract_skills_index(text: &str) -> Vec<SkillIndexEntry> {
    let mut skills = Vec::new();
    let mut in_available = false;

    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line == "### Available skills" {
            in_available = true;
            continue;
        }
        if in_available && line.starts_with("### ") {
            break;
        }
        if !in_available || !line.starts_with("- ") {
            continue;
        }

        let Some((name, description)) = line[2..].split_once(':') else {
            continue;
        };
        let clean_description = description
            .split(" (file:")
            .next()
            .unwrap_or(description)
            .trim();
        if !name.trim().is_empty() && !clean_description.is_empty() {
            skills.push(SkillIndexEntry {
                name: name.trim().to_string(),
                description: shorten(clean_description, 260),
            });
        }
    }

    skills
}

fn extract_relevant_memory_lines(text: &str) -> Vec<String> {
    const KEYWORDS: &[&str] = &[
        "cliproxyapi",
        "Antigravity",
        "apply_patch",
        "Gemini",
        "token",
        "debug_exchanges",
        "v1internal",
        "Codex",
        "protocol",
        "streaming",
    ];

    text.lines()
        .map(str::trim)
        .filter(|line| {
            !line.is_empty()
                && KEYWORDS
                    .iter()
                    .any(|keyword| line.to_lowercase().contains(&keyword.to_lowercase()))
        })
        .map(|line| shorten(line, 500))
        .collect()
}

fn summarize_permissions(text: &str) -> String {
    let mut lines = Vec::new();
    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        let lower = line.to_lowercase();
        if lower.contains("sandbox")
            || lower.contains("writable")
            || lower.contains("network access")
            || lower.contains("require_escalated")
            || lower.contains("approval")
            || lower.contains("approved command prefixes")
            || lower.contains("prefix_rule")
        {
            lines.push(line.to_string());
        }
        if lines.len() >= 36 {
            break;
        }
    }

    if lines.is_empty() {
        shorten(text, MAX_PERMISSIONS_CHARS)
    } else {
        shorten(&lines.join("\n"), MAX_PERMISSIONS_CHARS)
    }
}

fn summarize_plugins(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|line| {
            line.contains("Plugin")
                || line.contains("Skill naming")
                || line.contains("MCP naming")
                || line.contains("Trigger rules")
                || line.contains("Relevance")
                || line.starts_with("- ")
        })
        .take(30)
        .collect::<Vec<_>>()
        .join("\n")
}

fn summarize_collaboration_mode(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|line| {
            !line.is_empty()
                && (line.contains("Default mode")
                    || line.contains("Plan mode")
                    || line.contains("prefer making reasonable assumptions")
                    || line.contains("Never write a multiple choice"))
        })
        .take(18)
        .collect::<Vec<_>>()
        .join("\n")
}

fn should_keep_as_additional_instruction(text: &str) -> bool {
    if text.is_empty() {
        return false;
    }
    if text.contains("========= MEMORY_SUMMARY BEGINS =========")
        || text.contains("### Available skills")
        || text.contains("<permissions instructions>")
    {
        return false;
    }
    true
}

fn collapse_blank_lines(text: &str) -> String {
    let mut out = String::new();
    let mut blank_count = 0;
    for line in text.lines() {
        if line.trim().is_empty() {
            blank_count += 1;
            if blank_count <= 1 {
                out.push('\n');
            }
            continue;
        }
        blank_count = 0;
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out.trim().to_string()
}

fn dedupe_skills(skills: Vec<SkillIndexEntry>) -> Vec<SkillIndexEntry> {
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();
    for skill in skills {
        if seen.insert(skill.name.to_lowercase()) {
            result.push(skill);
        }
    }
    result.sort_by(|a, b| {
        let a_find = a.name.contains("find-skills");
        let b_find = b.name.contains("find-skills");
        b_find.cmp(&a_find).then_with(|| a.name.cmp(&b.name))
    });
    result
}

fn dedupe_lines(lines: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();
    for line in lines {
        let key = line.to_lowercase();
        if seen.insert(key) {
            result.push(line);
        }
    }
    result
}

fn shorten(text: &str, max_chars: usize) -> String {
    let mut count = 0;
    let mut out = String::new();
    for ch in text.chars() {
        if count >= max_chars {
            out.push_str("\n[truncated]");
            break;
        }
        out.push(ch);
        count += 1;
    }
    out
}
