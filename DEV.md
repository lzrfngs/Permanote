## DEV — Permanote Change Log

_Newest version first. Drop raw notes in the Inbox; I process them into the next version below._

<!-- Conventions: small version bumps on the third decimal (v0.1.1, v0.1.2 …). Strike shipped work with ~~strikethrough~~ and a short "what changed" note. Status markers: [x] done · [!] dropped/changed · [ ] open. -->

> Concept + full spec live in `Vibe_Coding/Permanote/KICKOFF.md`. This doc is the running memory of what's actually been built.

## 📥 Inbox

_Add new notes/requests here as bullets. They get processed into a version below, then cleared._

-
-

---

## v0.1.1 — Editor + palette polish

- [x] Smart slash-menu positioning: `/` command menu flips above the cursor when it would overflow the viewport bottom; clamps horizontally.
- [x] Multi-line paste into lists: pasting plain text with line breaks inside a bullet/checkbox list splits each line into its own list item (preserves bullet/checkbox). Skips when HTML clipboard data is present or there's only one line.
- [x] Canvas layout refactor: writing canvas moved to flex column with sticky tools (was absolute-positioned tools + margin centering); top padding 4rem → 1.5rem.
- [x] Permanote palette titles left-justified (`.perma-title` now `flex: 1` + `text-align: left`; previously floated centered in the space-between flex row).

---

## v0.1.0 — Initial MVP (tagged)

Stack as specced: Tauri 2 + Rust backend, SvelteKit + Svelte 5 front end, Tiptap 3 (ProseMirror) editor, SQLite FTS5 index via the Rust side.

**Backend (Rust, `src-tauri/src/`)**
- [x] Vault layer (`vault.rs`): read/write day files, list days, todos (with due-date scheduling), permanotes CRUD + backlinks.
- [x] FTS5 search index (`index.rs`): index-on-write, full `search`, `rebuild_index`.
- [x] OneDrive-safe file handling + file watcher (`watcher.rs`) with snapshot reconciliation.
- [x] Settings + first-run flow (`settings.rs`): vault location, default path, restart.
- [x] Tauri commands wired: `today`, `read_day`, `write_day`, `vault_path`, `open_vault_folder`, `list_todos`, `set_todo_state`, `set_todo_due`, `list_permanotes`, `read_permanote`, `write_permanote`, `delete_permanote`, `list_permanote_backlinks`, `list_days`, `search`, `rebuild_index`, `get_settings`, `update_settings`, `is_first_run`, `default_vault_path`, `restart_app`.

**Front end (Svelte, `src/`)**
- [x] Writing canvas with Tiptap: bold/italic/strike/headings/lists/checkboxes.
- [x] Custom nodes: permanote block (`permanote-node.ts`), permanote link (`permanote-link-node.ts`).
- [x] Slash commands (`slash-command.ts`).
- [x] Todos panel with scheduling, word count, slash-command cheatsheet, icons, general polish.

**Known gaps / not yet built (from KICKOFF MVP scope)**
- [ ] Calendar panel (month view) with content/unresolved-todo day indicators.
- [ ] Focus mode (collapse all side panels to a single shortcut).
- [ ] Conflict-copy tray (OneDrive `(... conflict).md` detection + merge/discard).
- [ ] Permanote two-way sync edge cases on OneDrive divergence (canonical = permanote file).

**Performance budget (treat as failing tests until verified)**
- [ ] Cold launch < 600 ms · day switch < 50 ms · search < 100 ms over 5y · idle RAM < 200 MB.
