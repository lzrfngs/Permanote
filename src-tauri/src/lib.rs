mod index;
mod settings;
mod vault;
mod watcher;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use chrono::Local;
use tauri::{Manager, State};

struct AppState {
    index: index::Index,
    snapshots: watcher::Snapshots,
    _watcher: Mutex<Option<notify::RecommendedWatcher>>,
}

#[tauri::command]
fn today() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

#[tauri::command]
fn read_day(date: String, state: State<'_, AppState>) -> Result<String, String> {
    let content = vault::read_day(&date)?;
    state
        .snapshots
        .lock()
        .unwrap()
        .insert(date.clone(), content.clone());
    Ok(content)
}

#[tauri::command]
fn write_day(
    date: String,
    content: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    vault::write_day(&date, &content)?;
    // Re-read so we index the id-injected canonical form on disk, not the
    // version the editor sent (which can lack `^t-XXXX` for new tasks).
    let stored = vault::read_day(&date).unwrap_or(content);
    let _ = state.index.index_day(&date, &stored);
    state
        .snapshots
        .lock()
        .unwrap()
        .insert(date.clone(), stored);
    Ok(())
}

#[tauri::command]
fn vault_path() -> Result<String, String> {
    vault::vault_root().map(|p| p.to_string_lossy().to_string())
}

#[tauri::command]
fn open_vault_folder() -> Result<(), String> {
    let root = vault::vault_root()?;
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(&root)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&root)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open")
            .arg(&root)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn list_todos() -> Result<Vec<vault::TodoItem>, String> {
    vault::list_todos()
}

#[tauri::command]
fn set_todo_state(
    date: String,
    line: usize,
    done: bool,
    state: State<'_, AppState>,
) -> Result<(), String> {
    vault::set_todo_state(&date, line, done)?;
    if let Ok(stored) = vault::read_day(&date) {
        let _ = state.index.index_day(&date, &stored);
        state
            .snapshots
            .lock()
            .unwrap()
            .insert(date.clone(), stored);
    }
    Ok(())
}

#[tauri::command]
fn set_todo_due(
    date: String,
    line: usize,
    due: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    vault::set_todo_due(&date, line, due.as_deref())?;
    if let Ok(stored) = vault::read_day(&date) {
        let _ = state.index.index_day(&date, &stored);
        state
            .snapshots
            .lock()
            .unwrap()
            .insert(date.clone(), stored);
    }
    Ok(())
}

#[tauri::command]
fn list_permanotes() -> Result<Vec<vault::PermanoteItem>, String> {
    vault::list_permanotes()
}

#[tauri::command]
fn read_permanote(id: String) -> Result<vault::PermanoteFile, String> {
    vault::read_permanote(&id)
}

#[tauri::command]
fn write_permanote(
    id: String,
    title: String,
    color: String,
    content: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    vault::write_permanote(&id, &title, &color, &content)?;
    // Refresh snapshot for the source day so the watcher doesn't fire a
    // self-write conflict banner.
    if let Ok(p) = vault::read_permanote(&id) {
        if !p.source_day.is_empty() {
            if let Ok(day_body) = vault::read_day(&p.source_day) {
                if let Ok(mut snaps) = state.snapshots.lock() {
                    snaps.insert(p.source_day.clone(), day_body);
                }
            }
        }
    }
    let _ = state.index.rebuild();
    Ok(())
}

#[tauri::command]
fn delete_permanote(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let source_day = vault::read_permanote(&id)
        .ok()
        .map(|p| p.source_day)
        .filter(|s| !s.is_empty());
    vault::delete_permanote(&id)?;
    if let Some(day) = source_day {
        if let Ok(day_body) = vault::read_day(&day) {
            if let Ok(mut snaps) = state.snapshots.lock() {
                snaps.insert(day, day_body);
            }
        }
    }
    let _ = state.index.rebuild();
    Ok(())
}

#[tauri::command]
fn list_permanote_backlinks(id: String) -> Result<Vec<String>, String> {
    vault::list_permanote_backlinks(&id)
}

#[tauri::command]
fn list_days() -> Result<Vec<vault::DayInfo>, String> {
    vault::list_days()
}

#[tauri::command]
fn search(
    query: String,
    state: State<'_, AppState>,
) -> Result<Vec<index::SearchHit>, String> {
    state.index.search(&query, 50)
}

#[tauri::command]
fn rebuild_index(state: State<'_, AppState>) -> Result<usize, String> {
    state.index.rebuild()
}

#[tauri::command]
fn get_settings() -> settings::Settings {
    settings::get()
}

#[tauri::command]
fn update_settings(new: settings::Settings) -> Result<(), String> {
    settings::save(new)
}

#[tauri::command]
fn is_first_run() -> bool {
    settings::is_first_run()
}

#[tauri::command]
fn default_vault_path() -> Result<String, String> {
    vault::default_vault_root().map(|p| p.to_string_lossy().to_string())
}

#[tauri::command]
fn restart_app(app: tauri::AppHandle) {
    app.restart();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let idx = index::Index::open().expect("failed to open index");
            let _ = idx.rebuild();
            let snapshots: watcher::Snapshots = Arc::new(Mutex::new(HashMap::new()));
            let watcher_handle = watcher::start(app.handle().clone(), snapshots.clone()).ok();
            app.manage(AppState {
                index: idx,
                snapshots,
                _watcher: Mutex::new(watcher_handle),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            today,
            read_day,
            write_day,
            vault_path,
            open_vault_folder,
            list_todos,
            set_todo_state,
            set_todo_due,
            list_permanotes,
            read_permanote,
            write_permanote,
            delete_permanote,
            list_permanote_backlinks,
            list_days,
            search,
            rebuild_index,
            get_settings,
            update_settings,
            is_first_run,
            default_vault_path,
            restart_app
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
