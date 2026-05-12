use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;

#[derive(Serialize)]
pub struct TodoItem {
    pub day: String,
    pub line: usize,
    pub id: String,
    pub text: String,
    pub done: bool,
    pub due: Option<String>,
}

#[derive(Serialize)]
pub struct PermanoteItem {
    pub id: String,
    pub day: String,
    pub line: usize,
    pub color: String,
    pub title: String,
    pub snippet: String,
}

#[derive(Serialize)]
pub struct DayInfo {
    pub date: String,
    pub has_open_todos: bool,
}

/// Returns the vault root. Created (with subfolders) if missing.
/// Path comes from settings; falls back to `Documents/Permanote` until the
/// user picks a folder via the first-run onboarding flow.
pub fn vault_root() -> Result<PathBuf, String> {
    let root = match crate::settings::get().vault_root {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => default_vault_root()?,
    };
    fs::create_dir_all(root.join("days")).map_err(|e| e.to_string())?;
    fs::create_dir_all(root.join("permanotes")).map_err(|e| e.to_string())?;
    fs::create_dir_all(root.join(".permanote")).map_err(|e| e.to_string())?;
    Ok(root)
}

pub fn default_vault_root() -> Result<PathBuf, String> {
    let docs = dirs::document_dir()
        .ok_or_else(|| "Could not locate Documents folder".to_string())?;
    Ok(docs.join("Permanote"))
}

pub fn day_path(date: &str) -> Result<PathBuf, String> {
    if !is_valid_date(date) {
        return Err(format!("Invalid date: {date}"));
    }
    Ok(vault_root()?.join("days").join(format!("{date}.md")))
}

pub fn days_dir() -> Result<PathBuf, String> {
    Ok(vault_root()?.join("days"))
}

fn is_valid_date(s: &str) -> bool {
    // YYYY-MM-DD, all digits with dashes in the right places
    let b = s.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b[..4].iter().all(|c| c.is_ascii_digit())
        && b[5..7].iter().all(|c| c.is_ascii_digit())
        && b[8..10].iter().all(|c| c.is_ascii_digit())
}

pub fn read_day(date: &str) -> Result<String, String> {
    let path = day_path(date)?;
    if !path.exists() {
        return Ok(default_day_content(date));
    }
    fs::read_to_string(&path).map_err(|e| e.to_string())
}

/// Atomic write: write to `.tmp`, fsync, rename over target.
/// Injects `^t-xxxx` ids on any task line missing one before saving so the
/// id is durable across reloads even though the editor never sees brand-new
/// tasks come back with an id attached.
pub fn write_day(date: &str, content: &str) -> Result<(), String> {
    let path = day_path(date)?;
    let injected = inject_todo_ids(content);
    write_atomic(&path, &injected)?;
    // Mirror every fenced permanote into permanotes/{id}.md so the library
    // exists as discrete files on disk.
    let _ = sync_permanote_files_from_day(date, &injected);
    Ok(())
}

pub fn permanotes_dir() -> Result<PathBuf, String> {
    Ok(vault_root()?.join("permanotes"))
}

pub fn permanote_path(id: &str) -> Result<PathBuf, String> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(format!("Invalid permanote id: {id}"));
    }
    Ok(permanotes_dir()?.join(format!("{id}.md")))
}

#[derive(Serialize, serde::Deserialize, Clone)]
pub struct PermanoteFile {
    pub id: String,
    pub title: String,
    pub color: String,
    pub source_day: String,
    pub created: String,
    pub modified: String,
    pub content: String,
}

/// Parse a day's markdown into a list of (id, title, color, body) for each
/// fenced permanote.
fn extract_permanote_fences(content: &str) -> Vec<(String, String, String, String)> {
    let lines: Vec<&str> = content.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if let Some(rest) = line
            .strip_prefix("%%permanote-start")
            .and_then(|s| s.strip_suffix("%%"))
            .map(|s| s.trim())
        {
            let attrs = parse_fence_attrs(rest);
            let id = attrs.get("id").cloned().unwrap_or_default();
            let color = attrs.get("color").cloned().unwrap_or_else(|| "amber".into());
            let title = attrs.get("title").cloned().unwrap_or_default();
            let mut body_lines: Vec<&str> = Vec::new();
            let mut j = i + 1;
            while j < lines.len() && !lines[j].starts_with("%%permanote-end") {
                body_lines.push(lines[j]);
                j += 1;
            }
            let body = body_lines.join("\n");
            if !id.is_empty() {
                out.push((id, title, color, body));
            }
            i = j + 1;
        } else {
            i += 1;
        }
    }
    out
}

fn now_iso() -> String {
    chrono::Local::now().to_rfc3339()
}

fn escape_yaml(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Write or update permanotes/{id}.md for every fenced permanote in a day.
/// Preserves `created` from the existing file if present; updates `modified`.
fn sync_permanote_files_from_day(date: &str, day_content: &str) -> Result<(), String> {
    let fences = extract_permanote_fences(day_content);
    for (id, title, color, body) in fences {
        let path = match permanote_path(&id) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let prior_title = if path.exists() {
            fs::read_to_string(&path)
                .ok()
                .and_then(|s| parse_yaml_field(&s, "title"))
        } else {
            None
        };
        let (created, prior_content) = if path.exists() {
            match fs::read_to_string(&path) {
                Ok(existing) => {
                    let created = parse_yaml_field(&existing, "created")
                        .unwrap_or_else(|| now_iso());
                    let prior_body = strip_frontmatter(&existing);
                    (created, prior_body)
                }
                Err(_) => (now_iso(), String::new()),
            }
        } else {
            (now_iso(), String::new())
        };
        // Skip rewrite if nothing meaningful changed.
        let existing_meta = if path.exists() {
            fs::read_to_string(&path).ok()
        } else {
            None
        };
        let same = existing_meta
            .as_ref()
            .map(|s| {
                parse_yaml_field(s, "title").as_deref() == Some(&title)
                    && parse_yaml_field(s, "color").as_deref() == Some(&color)
                    && parse_yaml_field(s, "source_day").as_deref() == Some(date)
                    && prior_content.trim() == body.trim()
            })
            .unwrap_or(false);
        if same {
            continue;
        }
        let modified = now_iso();
        let file = format!(
            "---\nid: {id}\ntitle: \"{title}\"\ncolor: {color}\nsource_day: {date}\ncreated: {created}\nmodified: {modified}\n---\n\n{body}\n",
            id = id,
            title = escape_yaml(&title),
            color = color,
            date = date,
            created = created,
            modified = modified,
            body = body.trim_end(),
        );
        write_atomic(&path, &file)?;
        // If the title changed, propagate to any `[[permanote:id|...]]`
        // backlinks across the vault so they stay in sync.
        if prior_title.as_deref() != Some(title.as_str()) {
            let _ = propagate_permanote_link_titles(&id, &title);
        }
    }
    Ok(())
}

/// Walk every day file and rewrite any `[[permanote:{id}|...]]` link to use
/// the supplied title. Returns silently on any individual file error.
fn propagate_permanote_link_titles(id: &str, new_title: &str) -> Result<(), String> {
    let days_dir = vault_root()?.join("days");
    let entries = match fs::read_dir(&days_dir) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };
    let needle = format!("[[permanote:{id}|");
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("md") {
            continue;
        }
        let body = match fs::read_to_string(&path) {
            Ok(b) => b,
            Err(_) => continue,
        };
        if !body.contains(&needle) {
            continue;
        }
        let updated = rewrite_permanote_links(&body, id, new_title);
        if updated != body {
            let _ = write_atomic(&path, &updated);
        }
    }
    Ok(())
}

fn rewrite_permanote_links(body: &str, id: &str, new_title: &str) -> String {
    let prefix = format!("[[permanote:{id}|");
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    while let Some(start) = rest.find(&prefix) {
        out.push_str(&rest[..start]);
        let after = &rest[start + prefix.len()..];
        match after.find("]]") {
            Some(end) => {
                out.push_str(&prefix);
                out.push_str(new_title);
                out.push_str("]]");
                rest = &after[end + 2..];
            }
            None => {
                // Malformed: copy rest verbatim and bail.
                out.push_str(&rest[start..]);
                return out;
            }
        }
    }
    out.push_str(rest);
    out
}

fn strip_frontmatter(s: &str) -> String {
    if let Some(rest) = s.strip_prefix("---\n") {
        if let Some(end) = rest.find("\n---\n") {
            return rest[end + 5..].trim_start().to_string();
        }
    }
    s.to_string()
}

fn parse_yaml_field(s: &str, key: &str) -> Option<String> {
    let rest = s.strip_prefix("---\n")?;
    let end = rest.find("\n---\n")?;
    let head = &rest[..end];
    for line in head.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix(&format!("{key}:")) {
            let v = v.trim();
            let v = v.strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or(v);
            return Some(v.to_string());
        }
    }
    None
}

pub fn read_permanote(id: &str) -> Result<PermanoteFile, String> {
    let path = permanote_path(id)?;
    let raw = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let title = parse_yaml_field(&raw, "title").unwrap_or_default();
    let color = parse_yaml_field(&raw, "color").unwrap_or_else(|| "amber".into());
    let source_day = parse_yaml_field(&raw, "source_day").unwrap_or_default();
    let created = parse_yaml_field(&raw, "created").unwrap_or_default();
    let modified = parse_yaml_field(&raw, "modified").unwrap_or_default();
    let content = strip_frontmatter(&raw);
    Ok(PermanoteFile {
        id: id.to_string(),
        title,
        color,
        source_day,
        created,
        modified,
        content: content.trim_end().to_string(),
    })
}

/// Write a permanote file from the Permanotes panel. Also rewrites the matching
/// fence in the source day's file so the two stay in sync.
pub fn write_permanote(
    id: &str,
    title: &str,
    color: &str,
    content: &str,
) -> Result<(), String> {
    let path = permanote_path(id)?;
    let (created, source_day, prior_title) = if path.exists() {
        let existing = fs::read_to_string(&path).map_err(|e| e.to_string())?;
        (
            parse_yaml_field(&existing, "created").unwrap_or_else(|| now_iso()),
            parse_yaml_field(&existing, "source_day").unwrap_or_default(),
            parse_yaml_field(&existing, "title"),
        )
    } else {
        (now_iso(), String::new(), None)
    };
    let modified = now_iso();
    let file = format!(
        "---\nid: {id}\ntitle: \"{title}\"\ncolor: {color}\nsource_day: {source_day}\ncreated: {created}\nmodified: {modified}\n---\n\n{body}\n",
        id = id,
        title = escape_yaml(title),
        color = color,
        source_day = source_day,
        created = created,
        modified = modified,
        body = content.trim_end(),
    );
    write_atomic(&path, &file)?;

    // Update the fence inside the source day file too.
    if !source_day.is_empty() {
        if let Ok(day_path) = day_path(&source_day) {
            if day_path.exists() {
                if let Ok(day_body) = fs::read_to_string(&day_path) {
                    let updated = rewrite_fence_in_day(&day_body, id, title, color, content);
                    if updated != day_body {
                        write_atomic(&day_path, &updated)?;
                    }
                }
            }
        }
    }

    // Keep inline backlinks (`[[permanote:id|title]]`) in sync if the title
    // changed.
    if prior_title.as_deref() != Some(title) {
        let _ = propagate_permanote_link_titles(id, title);
    }
    Ok(())
}

fn rewrite_fence_in_day(day: &str, id: &str, title: &str, color: &str, content: &str) -> String {
    let lines: Vec<&str> = day.lines().collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len() + 4);
    let mut i = 0;
    let target_id = id;
    while i < lines.len() {
        let line = lines[i];
        let is_match = line
            .strip_prefix("%%permanote-start")
            .and_then(|s| s.strip_suffix("%%"))
            .map(|s| {
                let attrs = parse_fence_attrs(s.trim());
                attrs.get("id").map(|v| v.as_str()) == Some(target_id)
            })
            .unwrap_or(false);
        if is_match {
            out.push(format!(
                "%%permanote-start id={id} color={color} title=\"{title}\"%%",
                id = id,
                color = color,
                title = escape_yaml(title),
            ));
            for body_line in content.lines() {
                out.push(body_line.to_string());
            }
            // Skip to the matching end fence.
            let mut j = i + 1;
            while j < lines.len() && !lines[j].starts_with("%%permanote-end") {
                j += 1;
            }
            out.push(format!("%%permanote-end id={id}%%", id = id));
            i = j + 1;
        } else {
            out.push(line.to_string());
            i += 1;
        }
    }
    let mut joined = out.join("\n");
    if day.ends_with('\n') && !joined.ends_with('\n') {
        joined.push('\n');
    }
    joined
}

fn write_atomic(path: &Path, content: &str) -> Result<(), String> {
    let tmp = path.with_extension("md.tmp");
    {
        let mut f = fs::File::create(&tmp).map_err(|e| e.to_string())?;
        f.write_all(content.as_bytes()).map_err(|e| e.to_string())?;
        f.sync_all().map_err(|e| e.to_string())?;
    }
    fs::rename(&tmp, path).map_err(|e| e.to_string())?;
    Ok(())
}

fn default_day_content(date: &str) -> String {
    format!("---\ndate: {date}\n---\n\n")
}

/// Scan every day file for GFM task list items and return them.
/// MVP: linear scan, regex-free, line-based. Locator is (day, line).
pub fn list_todos() -> Result<Vec<TodoItem>, String> {
    let days_dir = vault_root()?.join("days");
    let mut out = Vec::new();

    let entries = match fs::read_dir(&days_dir) {
        Ok(e) => e,
        Err(_) => return Ok(out),
    };

    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("md"))
        .collect();
    files.sort();

    for path in files {
        let stem = match path.file_stem().and_then(|s| s.to_str()) {
            Some(s) if is_valid_date(s) => s.to_string(),
            _ => continue,
        };
        let body = match fs::read_to_string(&path) {
            Ok(b) => b,
            Err(_) => continue,
        };
        for (idx, line) in body.lines().enumerate() {
            if let Some((done, id, text)) = parse_task_line(line) {
                let (clean, due) = extract_due(text);
                out.push(TodoItem {
                    day: stem.clone(),
                    line: idx,
                    id,
                    text: clean,
                    done,
                    due,
                });
            }
        }
    }
    Ok(out)
}

/// Recognise `- [ ] text`, `- [x] text`, `* [ ] text`, etc.
/// Returns (done, id, text). If the body starts with `^t-XXXX `, the id is
/// extracted and stripped from the returned text. Id is empty otherwise.
fn parse_task_line(line: &str) -> Option<(bool, String, &str)> {
    let trimmed = line.trim_start();
    let bytes = trimmed.as_bytes();
    if bytes.len() < 6 {
        return None;
    }
    let marker = bytes[0];
    if marker != b'-' && marker != b'*' && marker != b'+' {
        return None;
    }
    if bytes[1] != b' ' || bytes[2] != b'[' || bytes[4] != b']' || bytes[5] != b' ' {
        return None;
    }
    let done = match bytes[3] {
        b' ' => false,
        b'x' | b'X' => true,
        _ => return None,
    };
    let rest = &trimmed[6..];
    if let Some(after_marker) = rest.strip_prefix("^t-") {
        let id_end = after_marker
            .find(|c: char| !c.is_ascii_hexdigit())
            .unwrap_or(after_marker.len());
        if id_end > 0 {
            let id = after_marker[..id_end].to_string();
            let after = &after_marker[id_end..];
            let text = after.strip_prefix(' ').unwrap_or(after);
            return Some((done, id, text));
        }
    }
    Some((done, String::new(), rest))
}

/// Extract a scheduled-date marker from a task body.
///
/// Recognises two forms anywhere in the trailing portion of the text:
///   * `📅 YYYY-MM-DD` (Obsidian Tasks convention)
///   * `@YYYY-MM-DD` (ASCII shorthand)
///
/// Returns the text with the marker removed (trimmed) and the parsed date,
/// or the original text plus `None` if no valid marker is found.
pub fn extract_due(text: &str) -> (String, Option<String>) {
    if let Some(pos) = text.find('\u{1F4C5}') {
        let after = text[pos + '\u{1F4C5}'.len_utf8()..].trim_start();
        if let Some(date) = take_iso_date(after) {
            let before = text[..pos].trim_end();
            let tail = after[10..].trim_start();
            let mut cleaned = String::with_capacity(before.len() + tail.len() + 1);
            cleaned.push_str(before);
            if !before.is_empty() && !tail.is_empty() {
                cleaned.push(' ');
            }
            cleaned.push_str(tail);
            return (cleaned.trim().to_string(), Some(date));
        }
    }
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'@' && (i == 0 || matches!(bytes[i - 1], b' ' | b'\t')) {
            let after = &text[i + 1..];
            if let Some(date) = take_iso_date(after) {
                let before = text[..i].trim_end();
                let tail = after[10..].trim_start();
                let mut cleaned = String::with_capacity(before.len() + tail.len() + 1);
                cleaned.push_str(before);
                if !before.is_empty() && !tail.is_empty() {
                    cleaned.push(' ');
                }
                cleaned.push_str(tail);
                return (cleaned.trim().to_string(), Some(date));
            }
        }
        i += 1;
    }
    (text.to_string(), None)
}

fn take_iso_date(s: &str) -> Option<String> {
    if s.len() < 10 {
        return None;
    }
    let cand = &s[..10];
    if !is_valid_date(cand) {
        return None;
    }
    if let Some(next) = s.as_bytes().get(10) {
        if next.is_ascii_digit() || *next == b'-' {
            return None;
        }
    }
    Some(cand.to_string())
}

/// Walk all task lines in `content` and inject a fresh `^t-XXXX` id for any
/// task that lacks one. Existing ids are left alone.
fn inject_todo_ids(content: &str) -> String {
    let mut out = String::with_capacity(content.len() + 64);
    let mut counter: u64 = 0;
    let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
    // First pass: collect ids already present so we don't collide.
    for line in content.lines() {
        if let Some((_d, id, _t)) = parse_task_line(line) {
            if !id.is_empty() {
                used.insert(id);
            }
        }
    }
    for line in content.split_inclusive('\n') {
        let (body, eol) = if let Some(s) = line.strip_suffix("\r\n") {
            (s, "\r\n")
        } else if let Some(s) = line.strip_suffix('\n') {
            (s, "\n")
        } else {
            (line, "")
        };
        match parse_task_line(body) {
            Some((_done, id, _text)) if id.is_empty() => {
                let leading = body.len() - body.trim_start().len();
                let prefix_end = leading + 6; // "- [ ] "
                let prefix = &body[..prefix_end];
                let rest = &body[prefix_end..];
                let mut new_id = gen_todo_id(rest, counter);
                while used.contains(&new_id) {
                    counter = counter.wrapping_add(1);
                    new_id = gen_todo_id(rest, counter);
                }
                used.insert(new_id.clone());
                counter = counter.wrapping_add(1);
                out.push_str(prefix);
                out.push_str("^t-");
                out.push_str(&new_id);
                out.push(' ');
                out.push_str(rest);
                out.push_str(eol);
            }
            _ => {
                out.push_str(body);
                out.push_str(eol);
            }
        }
    }
    out
}

fn gen_todo_id(seed: &str, counter: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let mut h: u64 = 0xcbf29ce484222325;
    for b in seed.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h ^= now.wrapping_add(counter);
    h = h.wrapping_mul(0x100000001b3);
    format!("{:04x}", (h ^ (h >> 16)) & 0xffff)
}

/// Toggle (or set) the done state of a single task line in a day file.
pub fn set_todo_state(date: &str, line_index: usize, done: bool) -> Result<(), String> {
    let path = day_path(date)?;
    let content = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let mut lines: Vec<String> = content.split_inclusive('\n').map(|s| s.to_string()).collect();
    if line_index >= lines.len() {
        return Err(format!("Line index out of range: {line_index}"));
    }
    let line = &lines[line_index];
    let (eol, body) = if let Some(stripped) = line.strip_suffix("\r\n") {
        ("\r\n", stripped)
    } else if let Some(stripped) = line.strip_suffix('\n') {
        ("\n", stripped)
    } else {
        ("", line.as_str())
    };
    if parse_task_line(body).is_none() {
        return Err(format!("Line {line_index} is not a task line"));
    }
    let leading_ws_len = body.len() - body.trim_start().len();
    let indent = &body[..leading_ws_len];
    let after_marker = &body[leading_ws_len + 6..];
    let marker_char = body.as_bytes()[leading_ws_len] as char;
    let new_box = if done { "[x]" } else { "[ ]" };
    let new_line = format!("{indent}{marker_char} {new_box} {after_marker}{eol}");
    lines[line_index] = new_line;
    let new_content: String = lines.concat();
    write_atomic(&path, &new_content)
}

/// Set or clear the scheduled-date marker on a todo line.
///
/// `due` is either `Some("YYYY-MM-DD")` to add/replace, or `None` to clear.
/// Existing markers (📅 or @-form) are stripped before adding the new one.
pub fn set_todo_due(date: &str, line_index: usize, due: Option<&str>) -> Result<(), String> {
    if let Some(d) = due {
        if !is_valid_date(d) {
            return Err(format!("Invalid date: {d}"));
        }
    }
    let path = day_path(date)?;
    let content = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let mut lines: Vec<String> = content.split_inclusive('\n').map(|s| s.to_string()).collect();
    if line_index >= lines.len() {
        return Err(format!("Line index out of range: {line_index}"));
    }
    let line = &lines[line_index];
    let (eol, body) = if let Some(stripped) = line.strip_suffix("\r\n") {
        ("\r\n", stripped)
    } else if let Some(stripped) = line.strip_suffix('\n') {
        ("\n", stripped)
    } else {
        ("", line.as_str())
    };
    let Some((done, id, text)) = parse_task_line(body) else {
        return Err(format!("Line {line_index} is not a task line"));
    };
    let leading_ws_len = body.len() - body.trim_start().len();
    let indent = &body[..leading_ws_len];
    let marker_char = body.as_bytes()[leading_ws_len] as char;
    let box_str = if done { "[x]" } else { "[ ]" };
    let id_prefix = if id.is_empty() { String::new() } else { format!("^t-{id} ") };
    let (cleaned, _existing) = extract_due(text);
    let mut new_body = format!("{indent}{marker_char} {box_str} {id_prefix}{cleaned}");
    if let Some(d) = due {
        if !new_body.ends_with(' ') {
            new_body.push(' ');
        }
        new_body.push_str("\u{1F4C5} ");
        new_body.push_str(d);
    }
    new_body.push_str(eol);
    lines[line_index] = new_body;
    let new_content: String = lines.concat();
    write_atomic(&path, &new_content)
}
pub fn list_permanotes() -> Result<Vec<PermanoteItem>, String> {
    let days_dir = vault_root()?.join("days");
    let mut out = Vec::new();

    let entries = match fs::read_dir(&days_dir) {
        Ok(e) => e,
        Err(_) => return Ok(out),
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("md"))
        .collect();
    files.sort();

    for path in files {
        let stem = match path.file_stem().and_then(|s| s.to_str()) {
            Some(s) if is_valid_date(s) => s.to_string(),
            _ => continue,
        };
        let body = match fs::read_to_string(&path) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let lines: Vec<&str> = body.lines().collect();
        let mut i = 0;
        while i < lines.len() {
            let line = lines[i];
            if let Some(rest) = line
                .strip_prefix("%%permanote-start")
                .and_then(|s| s.strip_suffix("%%"))
                .map(|s| s.trim())
            {
                let attrs = parse_fence_attrs(rest);
                let id = attrs.get("id").cloned().unwrap_or_default();
                let color = attrs.get("color").cloned().unwrap_or_else(|| "amber".into());
                let title = attrs.get("title").cloned().unwrap_or_default();
                // First non-empty line of body becomes snippet.
                let mut snippet = String::new();
                let mut j = i + 1;
                while j < lines.len() {
                    if lines[j].starts_with("%%permanote-end") {
                        break;
                    }
                    let t = lines[j].trim();
                    if !t.is_empty() && snippet.is_empty() {
                        snippet = t.chars().take(120).collect();
                    }
                    j += 1;
                }
                out.push(PermanoteItem {
                    id,
                    day: stem.clone(),
                    line: i,
                    color,
                    title,
                    snippet,
                });
                i = j + 1;
            } else {
                i += 1;
            }
        }
    }
    Ok(out)
}

fn parse_fence_attrs(input: &str) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Skip whitespace.
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        // Read key.
        let key_start = i;
        while i < bytes.len() && bytes[i] != b'=' && !bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if key_start == i || i >= bytes.len() || bytes[i] != b'=' {
            break;
        }
        let key = &input[key_start..i];
        i += 1; // skip '='
        // Read value (quoted or bare).
        let value = if i < bytes.len() && bytes[i] == b'"' {
            i += 1;
            let v_start = i;
            while i < bytes.len() && bytes[i] != b'"' {
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    i += 2;
                } else {
                    i += 1;
                }
            }
            let v = &input[v_start..i];
            if i < bytes.len() {
                i += 1; // closing quote
            }
            v.replace("\\\"", "\"")
        } else {
            let v_start = i;
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            input[v_start..i].to_string()
        };
        out.insert(key.to_string(), value);
    }
    out
}

/// Enumerate all valid day files. Reports whether each day has any unresolved
/// task lines so the calendar can show a second indicator dot.
pub fn list_days() -> Result<Vec<DayInfo>, String> {
    let days_dir = vault_root()?.join("days");
    let mut out = Vec::new();
    let entries = match fs::read_dir(&days_dir) {
        Ok(e) => e,
        Err(_) => return Ok(out),
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("md"))
        .collect();
    files.sort();
    for path in files {
        let stem = match path.file_stem().and_then(|s| s.to_str()) {
            Some(s) if is_valid_date(s) => s.to_string(),
            _ => continue,
        };
        let body = fs::read_to_string(&path).unwrap_or_default();
        // A day file with no content beyond its frontmatter shouldn't show up
        // on the calendar — treat it the same as a day that was never opened.
        if strip_frontmatter(&body).trim().is_empty() {
            continue;
        }
        let mut has_open = false;
        for line in body.lines() {
            if let Some((done, _id, _text)) = parse_task_line(line) {
                if !done {
                    has_open = true;
                    break;
                }
            }
        }
        out.push(DayInfo {
            date: stem,
            has_open_todos: has_open,
        });
    }
    Ok(out)
}

/// Delete a permanote: removes permanotes/{id}.md and unwraps the matching
/// fenced block in every day file (preserving its inner content as plain
/// text). The card disappears from the Permanotes panel, but the words
/// originally inside it remain in the day where they were written.
pub fn delete_permanote(id: &str) -> Result<(), String> {
    let path = permanote_path(id)?;
    let source_day = if path.exists() {
        let raw = fs::read_to_string(&path).map_err(|e| e.to_string())?;
        let sd = parse_yaml_field(&raw, "source_day").unwrap_or_default();
        fs::remove_file(&path).map_err(|e| e.to_string())?;
        sd
    } else {
        String::new()
    };

    // Scan every day file for fences carrying this id and unwrap them,
    // and convert any inline `[[permanote:id|title]]` backlinks into the
    // bare title so nothing is left dangling.
    let link_prefix = format!("[[permanote:{id}|");
    let days_dir = vault_root()?.join("days");
    if let Ok(entries) = fs::read_dir(&days_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension().and_then(|s| s.to_str()) != Some("md") {
                continue;
            }
            let stem = match p.file_stem().and_then(|s| s.to_str()) {
                Some(s) if is_valid_date(s) => s.to_string(),
                _ => continue,
            };
            let body = match fs::read_to_string(&p) {
                Ok(b) => b,
                Err(_) => continue,
            };
            if stem != source_day && !body.contains(id) {
                continue;
            }
            let mut updated = unwrap_fence_in_day(&body, id);
            if updated.contains(&link_prefix) {
                updated = unwrap_permanote_links(&updated, id);
            }
            if updated != body {
                write_atomic(&p, &updated)?;
            }
        }
    }
    Ok(())
}

/// Replace every `[[permanote:{id}|TITLE]]` with just `TITLE` so deleted
/// permanotes don't leave dangling links behind.
fn unwrap_permanote_links(body: &str, id: &str) -> String {
    let prefix = format!("[[permanote:{id}|");
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    while let Some(start) = rest.find(&prefix) {
        out.push_str(&rest[..start]);
        let after = &rest[start + prefix.len()..];
        match after.find("]]") {
            Some(end) => {
                out.push_str(&after[..end]);
                rest = &after[end + 2..];
            }
            None => {
                out.push_str(&rest[start..]);
                return out;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Strip the `%%permanote-start%%` and `%%permanote-end%%` markers for the
/// given id while preserving the lines between them as plain text.
fn unwrap_fence_in_day(day: &str, id: &str) -> String {
    let lines: Vec<&str> = day.lines().collect();
    let trailing_newline = day.ends_with('\n');
    let mut out: Vec<&str> = Vec::with_capacity(lines.len());
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let is_start = line
            .strip_prefix("%%permanote-start")
            .and_then(|s| s.strip_suffix("%%"))
            .map(|s| {
                let attrs = parse_fence_attrs(s.trim());
                attrs.get("id").map(|v| v.as_str()) == Some(id)
            })
            .unwrap_or(false);
        if is_start {
            // Skip the start line itself.
            i += 1;
            // Optionally consume one blank line that immediately follows the
            // start marker so the unwrapped text doesn't get extra padding.
            if i < lines.len() && lines[i].trim().is_empty() {
                i += 1;
            }
            // Copy body lines until the matching end marker.
            while i < lines.len() {
                let l = lines[i];
                let is_end = l
                    .strip_prefix("%%permanote-end")
                    .and_then(|s| s.strip_suffix("%%"))
                    .map(|s| {
                        let attrs = parse_fence_attrs(s.trim());
                        attrs.get("id").map(|v| v.as_str()) == Some(id)
                    })
                    .unwrap_or(false);
                if is_end {
                    i += 1;
                    break;
                }
                out.push(l);
                i += 1;
            }
            // Trim a single blank line that the body left dangling before
            // the end marker, to keep paragraph spacing tidy.
            if let Some(last) = out.last() {
                if last.trim().is_empty() {
                    out.pop();
                }
            }
        } else {
            out.push(line);
            i += 1;
        }
    }
    let mut joined = out.join("\n");
    if trailing_newline && !joined.ends_with('\n') {
        joined.push('\n');
    }
    joined
}

/// Find every day file that references this permanote either via fence or
/// `[[permanote:id...]]` link, excluding the source day.
pub fn list_permanote_backlinks(id: &str) -> Result<Vec<String>, String> {
    let days_dir = vault_root()?.join("days");
    let mut out = Vec::new();
    let entries = match fs::read_dir(&days_dir) {
        Ok(e) => e,
        Err(_) => return Ok(out),
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("md"))
        .collect();
    files.sort();
    let needle_link = format!("[[permanote:{id}");
    for path in files {
        let stem = match path.file_stem().and_then(|s| s.to_str()) {
            Some(s) if is_valid_date(s) => s.to_string(),
            _ => continue,
        };
        let body = match fs::read_to_string(&path) {
            Ok(b) => b,
            Err(_) => continue,
        };
        if !body.contains(&needle_link) {
            continue;
        }
        out.push(stem);
    }
    out.sort_by(|a, b| b.cmp(a));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_validation() {
        // Shape check only — doesn't validate month/day ranges.
        assert!(is_valid_date("2026-05-12"));
        assert!(is_valid_date("0000-00-00"));
        assert!(!is_valid_date("26-05-12"));
        assert!(!is_valid_date("2026/05/12"));
        assert!(!is_valid_date("2026-5-12"));
        assert!(!is_valid_date("2026-05-12X"));
    }

    #[test]
    fn strip_frontmatter_removes_block() {
        let src = "---\ntitle: x\n---\nhello\n";
        assert_eq!(strip_frontmatter(src), "hello\n");
    }

    #[test]
    fn strip_frontmatter_noop_without_block() {
        assert_eq!(strip_frontmatter("hello\n"), "hello\n");
    }

    #[test]
    fn parse_yaml_field_basic() {
        let fm = "---\ntitle: \"Hello\"\ncolor: amber\n---\nbody\n";
        assert_eq!(parse_yaml_field(fm, "title").as_deref(), Some("Hello"));
        assert_eq!(parse_yaml_field(fm, "color").as_deref(), Some("amber"));
        assert_eq!(parse_yaml_field(fm, "missing"), None);
    }

    #[test]
    fn extract_due_emoji_marker() {
        let (clean, due) = extract_due("ship the thing \u{1F4C5} 2026-05-12");
        assert_eq!(due.as_deref(), Some("2026-05-12"));
        assert_eq!(clean, "ship the thing");
    }

    #[test]
    fn extract_due_ascii_marker() {
        let (clean, due) = extract_due("call dentist @2026-06-01 important");
        assert_eq!(due.as_deref(), Some("2026-06-01"));
        assert_eq!(clean, "call dentist important");
    }

    #[test]
    fn extract_due_no_marker() {
        let (clean, due) = extract_due("just a thing");
        assert_eq!(due, None);
        assert_eq!(clean, "just a thing");
    }

    #[test]
    fn extract_due_rejects_non_date_token() {
        // The function relies on is_valid_date's shape check.
        let (_clean, due) = extract_due("nope @abcd-ef-gh");
        assert_eq!(due, None);
    }

    #[test]
    fn extract_due_ignores_email() {
        let (_clean, due) = extract_due("ping foo@bar.com about 2026-05-12");
        // The `@` is glued to a word, not preceded by whitespace, so it's not a marker.
        assert_eq!(due, None);
    }

    #[test]
    fn rewrite_permanote_links_updates_titles() {
        let body = "see [[permanote:abc|Old]] and [[permanote:other|Keep]]";
        let out = rewrite_permanote_links(body, "abc", "New");
        assert_eq!(out, "see [[permanote:abc|New]] and [[permanote:other|Keep]]");
    }

    #[test]
    fn rewrite_permanote_links_noop_when_missing() {
        let body = "no link here";
        assert_eq!(rewrite_permanote_links(body, "abc", "New"), body);
    }

    #[test]
    fn unwrap_permanote_links_removes_only_target() {
        let body = "see [[permanote:abc|Title]] and [[permanote:xyz|Other]]";
        let out = unwrap_permanote_links(body, "abc");
        assert_eq!(out, "see Title and [[permanote:xyz|Other]]");
    }

    #[test]
    fn unwrap_fence_in_day_extracts_inner_content() {
        let day = "before\n%%permanote-start id=abc title=\"T\" color=amber%%\n\ninner text\n%%permanote-end id=abc%%\nafter\n";
        let out = unwrap_fence_in_day(day, "abc");
        assert!(out.contains("before"));
        assert!(out.contains("inner text"));
        assert!(out.contains("after"));
        assert!(!out.contains("permanote-start"));
        assert!(!out.contains("permanote-end"));
    }

    #[test]
    fn unwrap_fence_in_day_noop_when_id_missing() {
        let day = "no fences here\n";
        assert_eq!(unwrap_fence_in_day(day, "abc"), day);
    }

    #[test]
    fn parse_task_line_with_id() {
        let (done, id, text) = parse_task_line("- [ ] ^t-abcd write tests").unwrap();
        assert!(!done);
        assert_eq!(id, "abcd");
        assert_eq!(text, "write tests");
    }

    #[test]
    fn parse_task_line_without_id() {
        let (done, id, text) = parse_task_line("- [x] done thing").unwrap();
        assert!(done);
        assert!(id.is_empty());
        assert_eq!(text, "done thing");
    }

    #[test]
    fn parse_task_line_rejects_non_task() {
        assert!(parse_task_line("- bullet").is_none());
        assert!(parse_task_line("# heading").is_none());
    }
}
