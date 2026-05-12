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
///
/// Hardcoded to personal OneDrive for now — Windows' Documents folder is
/// redirected to the Microsoft work OneDrive on this machine, and Permanote
/// is a personal journal. Replace with a settings-file lookup when in-app
/// vault picker lands.
pub fn vault_root() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or_else(|| "Could not locate home folder".to_string())?;
    let root = home.join("OneDrive").join("Permanote");
    fs::create_dir_all(root.join("days")).map_err(|e| e.to_string())?;
    fs::create_dir_all(root.join("permanotes")).map_err(|e| e.to_string())?;
    fs::create_dir_all(root.join(".permanote")).map_err(|e| e.to_string())?;
    Ok(root)
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
    }
    Ok(())
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
    let (created, source_day) = if path.exists() {
        let existing = fs::read_to_string(&path).map_err(|e| e.to_string())?;
        (
            parse_yaml_field(&existing, "created").unwrap_or_else(|| now_iso()),
            parse_yaml_field(&existing, "source_day").unwrap_or_default(),
        )
    } else {
        (now_iso(), String::new())
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
                out.push(TodoItem {
                    day: stem.clone(),
                    line: idx,
                    id,
                    text: text.to_string(),
                    done,
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

/// Scan every day file for permanote fences.
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
