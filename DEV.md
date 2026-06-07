## DEV — Permanote Change Log

_Newest version first. Drop raw notes in the Inbox; I process them into the next version below._

<!-- Conventions: small version bumps on the third decimal (v0.1.1, v0.1.2 …). Strike shipped work with ~~strikethrough~~ and a short "what changed" note. Status markers: [x] done · [!] dropped/changed · [ ] open. -->

> Concept + full spec live in `Vibe_Coding/Permanote/KICKOFF.md`. This doc is the running memory of what's actually been built.

## 📥 Inbox

_Add new notes/requests here as bullets. They get processed into a version below, then cleared._

-
-

---

## v0.2.0 — Security, CRLF correctness, auto-updates

Security
- [x] CSP enabled via SvelteKit hash mode (`svelte.config.js`); inline bootstrap script auto-hashed per build, so it survives rebuilds. Tauri `csp` left null to defer to the meta tag. Was previously `null` (no policy).

Bug fixes (CRLF/line-ending class + robustness)
- [x] `normalize_lf()` added in `vault.rs`; `read_day`, `strip_frontmatter`, `parse_yaml_field`, and the three todo functions now normalize `\r\n`/`\r` → `\n` so OneDrive/Notepad-edited files parse correctly.
- [x] Frontend `splitFrontmatter()` now CRLF-tolerant.
- [x] Todo line-index desync fixed: list + mutate paths agree on line boundaries after normalization.
- [x] YAML field parser round-trips quotes/backslashes (`unescape_yaml`); titles with `"` no longer corrupt.
- [x] Watcher mutex no longer panics on poison (`lock().unwrap_or_else(|e| e.into_inner())`).
- [x] `onMount` editor/data wiring wrapped in try/catch; surfaces a toast + error status instead of a silent broken state.
- [x] Build fix: `$from` destructure (reserved `$` prefix in Svelte 5) was breaking `vite build`; renamed to `fromPos`.

Auto-update system (Tauri 2 updater plugin)
- [x] `tauri-plugin-updater` + `tauri-plugin-process` added (Cargo + npm), registered in `lib.rs` under `#[cfg(desktop)]`.
- [x] `createUpdaterArtifacts: true`, updater config + pubkey + endpoint in `tauri.conf.json`; endpoint = personal GH releases `latest.json`.
- [x] Capabilities: `updater:default`, `process:allow-restart`.
- [x] Ed25519 signing key generated at `~/.tauri/permanote.key` (no password, outside repo). Public key embedded in config.
- [x] In-app non-blocking "Update & restart" banner; flushes unsaved edits before install.
- [x] `.github/workflows/release.yml` (`tauri-action`, Windows, on `v*` tag) — runs on **personal** GH (`lzrfngs`) only; EMU account blocks Actions.
- [x] Versions bumped to 0.2.0 across package.json / Cargo.toml / tauri.conf.json.

### 🔑 Release runbook (manual one-time setup before first auto-update works)

1. On `github.com/lzrfngs/Permanote` → Settings → Secrets and variables → Actions, add:
   - `TAURI_SIGNING_PRIVATE_KEY` = contents of `~/.tauri/permanote.key`
   - `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` = empty string (key has no password)
2. Back up `~/.tauri/permanote.key` somewhere safe (password manager / encrypted). **If lost, existing installs can never receive updates.**
3. To cut a release: bump version in the 3 manifests, commit, then `git tag v0.2.0 && git push origin v0.2.0`. The workflow builds, signs, generates `latest.json`, and creates a **draft** release. Publish the draft to go live.
4. Installed apps check the endpoint on launch and show the update banner when a newer version is published.

Notes: OS code signing deferred (SmartScreen shows a one-time "Run anyway" on first browser download; auto-updates bypass it). The updater's Ed25519 signature is separate from OS code signing.

### Still open (deferred from this pass)
- [ ] Snapshot/watcher race on permanote write (med) — left as-is; rebuild() + snapshot refresh mitigates in practice.
- [ ] Split 3000-line `+page.svelte` and 1000-line `vault.rs` into modules (med, larger refactor).
- [ ] Magic strings → constants; dedupe fence parsing; remove dead unwrap fns; replace startup `.expect()` panics (low).

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

**Already built (corrected after 2026-06-06 audit)**
- [x] Calendar panel (month view) with content/open-todo day indicators, arrow-key nav + `T`.
- [x] Focus mode (`Ctrl+\` collapses side panels to center canvas).

**Known gaps / not yet built (from KICKOFF MVP scope)**
- [ ] Conflict-copy tray (OneDrive `(... conflict).md` detection + merge/discard). Watcher handles active-day external change banner only.
- [ ] Permanote two-way sync precedence on OneDrive divergence (canonical = permanote file).
- [ ] Backlinks footer inline on canvas (backend `list_permanote_backlinks` exists; only shown in detail modal).
- [ ] Slash set missing `/date` stamp + code block.
- [ ] Search UI: keyboard result nav, empty state, permanote-aware search (currently days only).

**Performance budget (treat as failing tests until verified)**
- [ ] Cold launch < 600 ms · day switch < 50 ms · search < 100 ms over 5y · idle RAM < 200 MB.

---

## Backlog — from 2026-06-06 multi-agent audit

### Security
- [ ] **CSP is `null`** in `tauri.conf.json` — set a restrictive policy. (high) Everything else (path traversal, SQLi, XSS, capabilities, atomic writes, shell) audited SAFE.

### Bugs (highest first)
- [ ] CRLF vs LF: `splitFrontmatter()` only matches `\n---\n`; OneDrive `\r\n` files break frontmatter split. (high, data risk)
- [ ] Todo line-index desync: `list_todos` uses `.lines()` while `set_todo_state` uses `split_inclusive('\n')` — can toggle wrong line on mixed EOL. (high)
- [ ] Snapshot race in `write_permanote`/`delete_permanote` vs watcher. (med)
- [ ] Mutex `.lock().unwrap()` poisoning panic in watcher. (med)
- [ ] `onMount` invoke() calls have no error handling → silent broken state. (med)
- [ ] Hand-rolled YAML field parser breaks on quotes/newlines in titles. (med)

### Code tidiness
- [ ] Split 3000-line `+page.svelte` into panel/editor/dialog components. (med, larger)
- [ ] Split 1000-line `vault.rs` into io/todo/permanote modules. (med, larger)
- [ ] Magic strings → constants (`%%permanote-start`, `^t-`, due emoji). (low, quick)
- [ ] Dedupe fence parsing; remove dead unwrap fns; replace startup `.expect()` panics. (low)

### Auto-update (Tauri 2 updater plugin — researched, viable)
- [ ] Add `tauri-plugin-updater` + `tauri-plugin-process`, Ed25519 signing key, `latest.json` via GitHub Releases on personal account (`lzrfngs`).
- [ ] `tauri-action` GitHub Actions workflow on tag push (personal GH only; EMU blocks Actions).
- [ ] Defer OS code signing (SmartScreen warning acceptable for personal distribution); updater signature is separate.

### Beyond-MVP feature ideas (aligned to minimal/local-first ethos)
- [ ] Permanote tags (frontmatter field exists, only color filter wired).
- [ ] WCAG AA contrast audit (owner requirement) — `.cal-dot.dot-open` #c08a3e on dark likely < 3:1.
- [ ] Permanote due dates + "upcoming reviews" rollup.
- [ ] Performance profiling against budget.
