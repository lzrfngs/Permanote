<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { Editor } from "@tiptap/core";
  import { Extension, InputRule } from "@tiptap/core";
  import StarterKit from "@tiptap/starter-kit";
  import TaskList from "@tiptap/extension-task-list";
  import TaskItem from "@tiptap/extension-task-item";
  import Placeholder from "@tiptap/extension-placeholder";
  import { Markdown } from "tiptap-markdown";
  import { Permanote } from "$lib/permanote-node";
  import { PermanoteLink } from "$lib/permanote-link-node";
  import { createSlashCommand, slashItems, type SlashItem, type SlashRenderProps } from "$lib/slash-command";

  // TaskItem with a stable id attribute. The id is written to disk as
  // `^t-XXXX` after the checkbox; it round-trips through markdown via the
  // post-load `migrateTaskIds` walk + the serializer-side regex in `save()`.
  const TaskItemWithId = TaskItem.extend({
    addAttributes() {
      return {
        ...this.parent?.(),
        tid: {
          default: null,
          parseHTML: (el: HTMLElement) => el.getAttribute("data-tid") || null,
          renderHTML: (attrs: any) =>
            attrs.tid ? { "data-tid": attrs.tid } : {},
        },
      };
    },
  });

  // Convert "[ ] " or "[x] " typed at the start of a paragraph or bullet item
  // into a task list item.
  const TaskListShortcut = Extension.create({
    name: "taskListShortcut",
    addInputRules() {
      return [
        new InputRule({
          find: /^\s*\[([ xX])\]\s$/,
          handler: ({ state, range, match, chain }) => {
            const checked = match[1].toLowerCase() === "x";
            const resolved = state.doc.resolve(range.from);
            let inListItem = false;
            for (let d = resolved.depth; d >= 0; d--) {
              if (resolved.node(d).type.name === "listItem") {
                inListItem = true;
                break;
              }
            }
            const c = chain().deleteRange(range);
            if (inListItem) {
              c.toggleList("taskList", "taskItem");
            } else {
              c.toggleTaskList();
            }
            c.updateAttributes("taskItem", { checked }).run();
          },
        }),
      ];
    },
  });

  let editorEl: HTMLDivElement;
  let editor: Editor | null = null;
  let date = $state("");
  let vaultPath = $state("");
  let status = $state<"idle" | "loading" | "saving" | "saved" | "error">("loading");
  let saveTimer: ReturnType<typeof setTimeout> | null = null;
  let wordCount = $state(0);

  function recomputeWordCount() {
    if (!editor) return;
    const text = editor.getText().trim();
    wordCount = text ? text.split(/\s+/).length : 0;
  }
  let frontmatter = "";
  let dirty = $state(false);
  let externalConflict = $state<{ date: string; content: string } | null>(null);
  let focusMode = $state(false);
  let unlistenExternal: UnlistenFn | null = null;

  // ── Settings & first-run ──────────────────────────────────────────────
  type Settings = {
    vault_root: string | null;
    permanote_mode: "color" | "label";
    theme: "light" | "dark" | "system";
    permanote_order: string[];
  };
  let settings = $state<Settings>({
    vault_root: null,
    permanote_mode: "color",
    theme: "system",
    permanote_order: [],
  });
  let firstRun = $state(false);
  let settingsOpen = $state(false);
  let shortcutsOpen = $state(false);
  let onboardingPath = $state("");
  let themeMql: MediaQueryList | null = null;
  function resolveTheme(t: Settings["theme"]): "light" | "dark" {
    if (t === "system") {
      return typeof window !== "undefined" &&
        window.matchMedia("(prefers-color-scheme: light)").matches
        ? "light"
        : "dark";
    }
    return t;
  }
  function applyTheme() {
    const resolved = resolveTheme(settings.theme);
    document.documentElement.dataset.theme = resolved;
    document.documentElement.style.colorScheme = resolved;
  }
  async function saveSettings(patch: Partial<Settings>) {
    settings = { ...settings, ...patch };
    await invoke("update_settings", { new: settings });
    applyTheme();
  }
  async function pickVaultFolder(): Promise<string | null> {
    const result = await openDialog({
      directory: true,
      multiple: false,
      title: "Choose your Permanote vault",
    });
    if (typeof result === "string") return result;
    return null;
  }
  async function confirmOnboarding() {
    if (!onboardingPath) return;
    await saveSettings({ vault_root: onboardingPath });
    await invoke("restart_app");
  }
  async function changeVault() {
    const picked = await pickVaultFolder();
    if (!picked) return;
    await saveSettings({ vault_root: picked });
    await invoke("restart_app");
  }

  type Todo = { day: string; line: number; id: string; text: string; done: boolean; due?: string | null };
  let todos = $state<Todo[]>([]);
  let todoFilter = $state<"open" | "done" | "scheduled" | "all">("open");

  const scheduledForToday = $derived.by(() => {
    if (!date) return [] as Todo[];
    return todos.filter((t) => !t.done && t.due === date && t.day !== date);
  });

  type Permanote = {
    id: string;
    day: string;
    line: number;
    color: string;
    title: string;
    snippet: string;
  };
  let permanotes = $state<Permanote[]>([]);
  let permaColorFilter = $state<string | "all">("all");
  let permaQuery = $state("");
  let permaSort = $state<"recent" | "title" | "manual">("recent");
  let permaFilterOpen = $state(false);
  let permaCollapsed = $state<Set<string>>(new Set());
  function permaKey(p: { day: string; id: string }) { return `${p.day}:${p.id}`; }
  function togglePermaCollapsed(p: { day: string; id: string }) {
    const k = permaKey(p);
    const next = new Set(permaCollapsed);
    if (next.has(k)) next.delete(k); else next.add(k);
    permaCollapsed = next;
  }

  const PERMA_COLORS = ["amber", "cobalt", "rose", "sage", "violet", "slate"] as const;

  const filteredPermanotes = $derived.by(() => {
    const q = permaQuery.trim().toLowerCase();
    let list = permanotes.filter((p) => {
      if (permaColorFilter !== "all" && p.color !== permaColorFilter) return false;
      if (q) {
        const hay = `${p.title || ""} ${p.snippet || ""}`.toLowerCase();
        if (!hay.includes(q)) return false;
      }
      return true;
    });
    if (permaSort === "title") {
      list = [...list].sort((a, b) =>
        (a.title || "").localeCompare(b.title || "")
      );
    } else if (permaSort === "manual") {
      const order = settings.permanote_order || [];
      const rank = new Map(order.map((id, i) => [id, i]));
      list = [...list].sort((a, b) => {
        const ra = rank.has(a.id) ? rank.get(a.id)! : -1;
        const rb = rank.has(b.id) ? rank.get(b.id)! : -1;
        // Items not in saved order float to the top (newest first by day desc).
        if (ra === -1 && rb === -1) return a.day < b.day ? 1 : a.day > b.day ? -1 : 0;
        if (ra === -1) return -1;
        if (rb === -1) return 1;
        return ra - rb;
      });
    } else {
      // recent: by day desc
      list = [...list].sort((a, b) => (a.day < b.day ? 1 : a.day > b.day ? -1 : 0));
    }
    return list;
  });

  let dragId = $state<string | null>(null);
  let dragOverId = $state<string | null>(null);
  function onPermaDragStart(e: DragEvent, id: string) {
    dragId = id;
    if (e.dataTransfer) {
      e.dataTransfer.effectAllowed = "move";
      e.dataTransfer.setData("text/plain", id);
    }
  }
  function onPermaDragOver(e: DragEvent, id: string) {
    if (!dragId || dragId === id) return;
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = "move";
    dragOverId = id;
  }
  function onPermaDragLeave(id: string) {
    if (dragOverId === id) dragOverId = null;
  }
  async function onPermaDrop(e: DragEvent, targetId: string) {
    e.preventDefault();
    const sourceId = dragId;
    dragId = null;
    dragOverId = null;
    if (!sourceId || sourceId === targetId) return;
    const visible = filteredPermanotes.map((p) => p.id);
    const from = visible.indexOf(sourceId);
    const to = visible.indexOf(targetId);
    if (from < 0 || to < 0) return;
    const next = [...visible];
    next.splice(from, 1);
    next.splice(to, 0, sourceId);
    // Merge with any ids not currently visible (filtered out): preserve their
    // existing relative order at the tail.
    const seen = new Set(next);
    const tail = (settings.permanote_order || []).filter((id) => !seen.has(id));
    const newOrder = [...next, ...tail];
    await saveSettings({ permanote_order: newOrder });
    if (permaSort !== "manual") permaSort = "manual";
  }
  function onPermaDragEnd() {
    dragId = null;
    dragOverId = null;
  }

  type DayInfo = { date: string; has_open_todos: boolean };
  let days = $state<DayInfo[]>([]);
  // Anchor month for the calendar grid: "YYYY-MM".
  let calendarMonth = $state("");

  async function refreshDays() {
    try {
      days = await invoke<DayInfo[]>("list_days");
    } catch (e) {
      console.error("list_days failed", e);
    }
  }

  function shiftMonth(delta: number) {
    const [y, m] = calendarMonth.split("-").map(Number);
    const dt = new Date(y, m - 1 + delta, 1);
    calendarMonth = `${dt.getFullYear()}-${String(dt.getMonth() + 1).padStart(2, "0")}`;
  }

  function shiftDay(delta: number) {
    if (!date) return;
    const [y, m, d] = date.split("-").map(Number);
    const dt = new Date(y, m - 1, d + delta);
    const next = `${dt.getFullYear()}-${String(dt.getMonth() + 1).padStart(2, "0")}-${String(dt.getDate()).padStart(2, "0")}`;
    jumpToDay(next);
  }

  async function jumpToToday() {
    const t = await invoke<string>("today");
    jumpToDay(t);
  }

  function onKeydown(e: KeyboardEvent) {
    // Ctrl/Cmd + \ toggles focus mode even from within the editor.
    if ((e.ctrlKey || e.metaKey) && e.key === "\\") {
      e.preventDefault();
      focusMode = !focusMode;
      return;
    }
    // Ctrl/Cmd + K focuses search.
    if ((e.ctrlKey || e.metaKey) && (e.key === "k" || e.key === "K")) {
      e.preventDefault();
      searchInputEl?.focus();
      searchInputEl?.select();
      return;
    }
    // Ctrl/Cmd + S flushes any pending save immediately.
    if ((e.ctrlKey || e.metaKey) && (e.key === "s" || e.key === "S")) {
      e.preventDefault();
      if (saveTimer) {
        clearTimeout(saveTimer);
        saveTimer = null;
      }
      if (dirty) save();
      return;
    }
    // Escape closes the cheatsheet from anywhere.
    if (e.key === "Escape" && shortcutsOpen) {
      e.preventDefault();
      shortcutsOpen = false;
      return;
    }
    // Only trigger if focus isn't inside an editable element / input.
    const target = e.target as HTMLElement | null;
    const tag = target?.tagName?.toLowerCase();
    if (
      tag === "input" ||
      tag === "textarea" ||
      target?.isContentEditable
    ) {
      return;
    }
    if (e.key === "ArrowLeft") {
      e.preventDefault();
      shiftDay(-1);
    } else if (e.key === "ArrowRight") {
      e.preventDefault();
      shiftDay(1);
    } else if (e.key === "t" || e.key === "T") {
      e.preventDefault();
      jumpToToday();
    } else if (e.key === "?") {
      e.preventDefault();
      shortcutsOpen = !shortcutsOpen;
    }
  }

  type SearchHit = { date: string; snippet: string };  let searchQuery = $state("");
  let searchHits = $state<SearchHit[]>([]);
  let searchInputEl: HTMLInputElement | null = $state(null);
  let searchTimer: ReturnType<typeof setTimeout> | null = null;

  type Toast = { id: number; message: string; kind: "error" | "info" };
  let toasts = $state<Toast[]>([]);
  let toastSeq = 0;
  function notify(message: string, kind: Toast["kind"] = "error") {
    const id = ++toastSeq;
    toasts = [...toasts, { id, message, kind }];
    setTimeout(() => {
      toasts = toasts.filter((t) => t.id !== id);
    }, 5000);
  }
  function dismissToast(id: number) {
    toasts = toasts.filter((t) => t.id !== id);
  }

  function onSearchInput() {
    if (searchTimer) clearTimeout(searchTimer);
    const q = searchQuery;
    searchTimer = setTimeout(async () => {
      if (!q.trim()) {
        searchHits = [];
        return;
      }
      try {
        searchHits = await invoke<SearchHit[]>("search", { query: q });
      } catch (e) {
        console.error("search failed", e);
        searchHits = [];
      }
    }, 120);
  }

  function clearSearch() {
    searchQuery = "";
    searchHits = [];
  }

  // Slash command menu state
  let slashOpen = $state(false);
  let slashItemsList = $state<SlashItem[]>([]);
  let slashIndex = $state(0);
  let slashLeft = $state(0);
  let slashTop = $state(0);
  let slashCommand: ((item: SlashItem) => void) | null = null;

  function pickSlash(i: number) {
    const item = slashItemsList[i];
    if (item && slashCommand) slashCommand(item);
  }

  const SlashCommand = createSlashCommand(() => ({
    onStart: (props: SlashRenderProps) => {
      slashItemsList = props.items;
      slashIndex = 0;
      slashCommand = props.command;
      const rect = props.clientRect?.();
      if (rect) {
        slashLeft = rect.left;
        slashTop = rect.bottom + 4;
      }
      slashOpen = true;
    },
    onUpdate: (props: SlashRenderProps) => {
      slashItemsList = props.items;
      slashIndex = Math.min(slashIndex, Math.max(0, props.items.length - 1));
      slashCommand = props.command;
      const rect = props.clientRect?.();
      if (rect) {
        slashLeft = rect.left;
        slashTop = rect.bottom + 4;
      }
    },
    onKeyDown: ({ event }) => {
      if (!slashOpen) return false;
      if (event.key === "ArrowDown") {
        slashIndex = (slashIndex + 1) % Math.max(1, slashItemsList.length);
        return true;
      }
      if (event.key === "ArrowUp") {
        slashIndex = (slashIndex - 1 + slashItemsList.length) % Math.max(1, slashItemsList.length);
        return true;
      }
      if (event.key === "Enter" || event.key === "Tab") {
        if (slashItemsList.length === 0) return false;
        pickSlash(slashIndex);
        return true;
      }
      if (event.key === "Escape") {
        slashOpen = false;
        return true;
      }
      return false;
    },
    onExit: () => {
      slashOpen = false;
      slashItemsList = [];
      slashCommand = null;
    },
  }));

  async function refreshTodos() {
    try {
      todos = await invoke<Todo[]>("list_todos");
    } catch (e) {
      console.error("list_todos failed", e);
    }
  }

  async function refreshPermanotes() {
    try {
      permanotes = await invoke<Permanote[]>("list_permanotes");
    } catch (e) {
      console.error("list_permanotes failed", e);
    }
  }

  type PermanoteFile = {
    id: string;
    title: string;
    color: string;
    source_day: string;
    created: string;
    modified: string;
    content: string;
  };
  let permaDetail = $state<PermanoteFile | null>(null);
  let permaDetailDraft = $state<{ title: string; color: string; content: string } | null>(null);
  let permaDetailBacklinks = $state<string[]>([]);
  let permaDetailSaving = $state(false);
  let permaDetailDeleting = $state(false);
  let permaDetailError = $state<string | null>(null);

  const permaDetailDirty = $derived.by(() => {
    if (!permaDetail || !permaDetailDraft) return false;
    return (
      permaDetailDraft.title !== permaDetail.title ||
      permaDetailDraft.color !== permaDetail.color ||
      permaDetailDraft.content !== permaDetail.content
    );
  });

  async function openPermanote(id: string) {
    permaDetailError = null;
    permaDetailBacklinks = [];
    try {
      const file = await invoke<PermanoteFile>("read_permanote", { id });
      permaDetail = file;
      permaDetailDraft = {
        title: file.title,
        color: file.color,
        content: file.content,
      };
      try {
        permaDetailBacklinks = await invoke<string[]>("list_permanote_backlinks", { id });
      } catch (e) {
        console.error("list_permanote_backlinks failed", e);
      }
    } catch (e) {
      console.error("read_permanote failed", e);
      permaDetailError = String(e);
    }
  }

  function closePermanote() {
    permaDetail = null;
    permaDetailDraft = null;
    permaDetailBacklinks = [];
    permaDetailError = null;
  }

  async function savePermanote() {
    if (!permaDetail || !permaDetailDraft) return;
    permaDetailSaving = true;
    permaDetailError = null;
    try {
      await invoke("write_permanote", {
        id: permaDetail.id,
        title: permaDetailDraft.title,
        color: permaDetailDraft.color,
        content: permaDetailDraft.content,
      });
      // Reload current day if it's the source day so the canvas reflects changes.
      if (permaDetail.source_day === date) {
        await loadDay();
      }
      await refreshPermanotes();
      // Refresh the file from disk so created/modified update.
      const fresh = await invoke<PermanoteFile>("read_permanote", { id: permaDetail.id });
      permaDetail = fresh;
      permaDetailDraft = {
        title: fresh.title,
        color: fresh.color,
        content: fresh.content,
      };
    } catch (e) {
      console.error("write_permanote failed", e);
      permaDetailError = String(e);
      notify(`Save failed: ${e}`);
    } finally {
      permaDetailSaving = false;
    }
  }

  async function deletePermanote() {
    if (!permaDetail) return;
    const ok = confirm(
      `Delete permanote "${permaDetail.title || "Untitled"}"? The card will be removed; the text inside it stays in ${permaDetail.source_day || "its source day"} as regular content.`,
    );
    if (!ok) return;
    permaDetailDeleting = true;
    permaDetailError = null;
    const sourceDay = permaDetail.source_day;
    try {
      await invoke("delete_permanote", { id: permaDetail.id });
      if (sourceDay === date) {
        await loadDay();
      }
      await refreshPermanotes();
      closePermanote();
    } catch (e) {
      console.error("delete_permanote failed", e);
      permaDetailError = String(e);
      notify(`Delete failed: ${e}`);
    } finally {
      permaDetailDeleting = false;
    }
  }

  // Permanote link picker (invoked from the slash command "Link permanote").
  let linkPickerOpen = $state(false);
  let linkPickerQuery = $state("");
  let linkPickerIndex = $state(0);
  const linkPickerHits = $derived.by(() => {
    const q = linkPickerQuery.trim().toLowerCase();
    const list = q
      ? permanotes.filter((p) =>
          (p.title || "").toLowerCase().includes(q) ||
          (p.snippet || "").toLowerCase().includes(q) ||
          p.id.includes(q),
        )
      : permanotes;
    return [...list]
      .sort((a, b) => (a.day < b.day ? 1 : a.day > b.day ? -1 : 0))
      .slice(0, 20);
  });

  function openLinkPicker() {
    linkPickerQuery = "";
    linkPickerIndex = 0;
    linkPickerOpen = true;
  }

  function closeLinkPicker() {
    linkPickerOpen = false;
  }

  function insertLinkFromPicker(p: Permanote) {
    if (!editor) return;
    editor
      .chain()
      .focus()
      .insertPermanoteLink({ id: p.id, title: p.title || "Untitled" })
      .run();
    closeLinkPicker();
  }

  function focusOnMount(node: HTMLElement) {
    queueMicrotask(() => node.focus());
  }

  function onLinkPickerKey(e: KeyboardEvent) {
    if (!linkPickerOpen) return;
    if (e.key === "ArrowDown") {
      e.preventDefault();
      linkPickerIndex = Math.min(linkPickerIndex + 1, Math.max(0, linkPickerHits.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      linkPickerIndex = Math.max(0, linkPickerIndex - 1);
    } else if (e.key === "Enter") {
      e.preventDefault();
      const hit = linkPickerHits[linkPickerIndex];
      if (hit) insertLinkFromPicker(hit);
    } else if (e.key === "Escape") {
      e.preventDefault();
      closeLinkPicker();
    }
  }

  async function toggleTodo(t: Todo) {
    const newDone = !t.done;
    try {
      await invoke("set_todo_state", { date: t.day, line: t.line, done: newDone });
      if (t.day === date) {
        // Reload editor so the canvas reflects the file change.
        await loadDay();
      }
      await refreshTodos();
    } catch (e) {
      console.error("set_todo_state failed", e);
      notify(`Toggle failed: ${e}`);
    }
  }

  let scheduleOpenKey = $state<string | null>(null);
  let scheduleDraft = $state("");
  function openSchedule(t: Todo) {
    const key = t.day + ":" + t.line;
    if (scheduleOpenKey === key) {
      scheduleOpenKey = null;
      return;
    }
    scheduleDraft = t.due ?? "";
    scheduleOpenKey = key;
  }
  async function setTodoDue(t: Todo, due: string | null) {
    try {
      await invoke("set_todo_due", { date: t.day, line: t.line, due });
      scheduleOpenKey = null;
      if (t.day === date) await loadDay();
      await refreshTodos();
    } catch (e) {
      console.error("set_todo_due failed", e);
      notify(`Schedule failed: ${e}`);
    }
  }
  function todayIso(): string {
    const d = new Date();
    return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
  }
  function tomorrowIso(): string {
    const d = new Date();
    d.setDate(d.getDate() + 1);
    return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
  }
  function dueLabel(d: string): string {
    const t = todayIso();
    if (d === t) return "Today";
    if (d === tomorrowIso()) return "Tomorrow";
    const [y, m, dd] = d.split("-").map(Number);
    const dt = new Date(y, m - 1, dd);
    return dt.toLocaleDateString(undefined, { month: "short", day: "numeric" });
  }

  async function jumpToDay(newDate: string) {
    if (newDate === date) return;
    if (saveTimer) {
      clearTimeout(saveTimer);
      saveTimer = null;
      await save();
    }
    date = newDate;
    await loadDay();
  }

  function splitFrontmatter(raw: string): { frontmatter: string; body: string } {
    if (raw.startsWith("---\n")) {
      const end = raw.indexOf("\n---\n", 4);
      if (end !== -1) {
        return { frontmatter: raw.slice(0, end + 5), body: raw.slice(end + 5) };
      }
    }
    return { frontmatter: `---\ndate: ${date}\n---\n\n`, body: raw };
  }

  function scheduleSave() {
    status = "saving";
    dirty = true;
    if (saveTimer) clearTimeout(saveTimer);
    saveTimer = setTimeout(save, 500);
  }

  async function save() {
    if (!editor) return;
    try {
      const body = (editor.storage as any).markdown.getMarkdown();
      // Collect taskItem tids in document order. Tiptap doesn't emit them
      // through markdown, so we inject them after the fact.
      const tids: (string | null)[] = [];
      editor.state.doc.descendants((node: any) => {
        if (node.type.name === "taskItem") {
          tids.push(node.attrs.tid || null);
          return false;
        }
        return true;
      });
      let i = 0;
      const withIds = body.replace(
        /^(\s*[-*+]\s+\[[ xX]\]\s+)(?!\^t-)/gm,
        (m0: string) => {
          const tid = tids[i++];
          return tid ? `${m0}^t-${tid} ` : m0;
        }
      );
      const content = frontmatter + withIds;
      await invoke("write_day", { date, content });
      status = "saved";
      dirty = false;
      refreshTodos();
      refreshPermanotes();
      refreshDays();
    } catch (e) {
      console.error(e);
      status = "error";
      notify(`Save failed: ${e}`);
    }
  }

  async function loadDay() {
    status = "loading";
    const raw = await invoke<string>("read_day", { date });
    const split = splitFrontmatter(raw);
    frontmatter = split.frontmatter;
    editor?.commands.setContent(split.body.trim() ? split.body : "", {
      emitUpdate: false,
    });
    migrateTaskIds();
    status = "saved";
    dirty = false;
    externalConflict = null;
    recomputeWordCount();
  }

  // After setContent, each taskItem's first text node may start with
  // `^t-XXXX `. Extract that into the node's `tid` attr and remove it from
  // the visible text. Idempotent: skips items that already have a tid.
  function migrateTaskIds() {
    if (!editor) return;
    type Mutation = { pos: number; tid: string; textPos: number; textLen: number };
    const mutations: Mutation[] = [];
    editor.state.doc.descendants((node: any, pos: number) => {
      if (node.type.name !== "taskItem") return true;
      if (node.attrs.tid) return false;
      let textNode: any = null;
      let textOffset = -1;
      node.descendants((child: any, offset: number) => {
        if (textNode) return false;
        if (child.isText) {
          textNode = child;
          textOffset = offset;
          return false;
        }
        return true;
      });
      if (!textNode) return false;
      const m = textNode.text.match(/^\^t-([0-9a-fA-F]+)\s/);
      if (!m) return false;
      mutations.push({
        pos,
        tid: m[1],
        textPos: pos + 1 + textOffset,
        textLen: m[0].length,
      });
      return false;
    });
    if (mutations.length === 0) return;
    const tr = editor.state.tr;
    // Apply in reverse so earlier positions are not disturbed.
    for (const mut of mutations.slice().reverse()) {
      tr.delete(mut.textPos, mut.textPos + mut.textLen);
      const node = tr.doc.nodeAt(mut.pos);
      if (node) {
        tr.setNodeMarkup(mut.pos, undefined, { ...node.attrs, tid: mut.tid });
      }
    }
    editor.view.dispatch(tr.setMeta("addToHistory", false));
  }

  onMount(async () => {
    // Load settings first so theme applies before any paint of the editor.
    settings = await invoke<Settings>("get_settings");
    applyTheme();
    themeMql = window.matchMedia("(prefers-color-scheme: light)");
    themeMql.addEventListener("change", applyTheme);

    firstRun = await invoke<boolean>("is_first_run");
    if (firstRun) {
      try {
        onboardingPath = await invoke<string>("default_vault_path");
      } catch {
        onboardingPath = "";
      }
      // Skip editor / data wiring until the user picks a vault and restarts.
      return;
    }

    date = await invoke<string>("today");
    vaultPath = await invoke<string>("vault_path");
    calendarMonth = date.slice(0, 7);
    window.addEventListener("keydown", onKeydown);
    window.addEventListener("keydown", onLinkPickerKey);
    window.addEventListener("permanote:open-link-picker", openLinkPicker as EventListener);

    editor = new Editor({
      element: editorEl,
      extensions: [
        StarterKit,
        TaskList,
        TaskItemWithId.configure({ nested: true }),
        TaskListShortcut,
        Permanote,
        PermanoteLink,
        SlashCommand,
        Placeholder.configure({
          placeholder: "Start writing… type / for commands",
          showOnlyWhenEditable: true,
          showOnlyCurrent: false,
        }),
        Markdown.configure({
          html: true,
          tightLists: true,
          bulletListMarker: "-",
          linkify: false,
          breaks: false,
          transformPastedText: true,
          transformCopiedText: false,
        }),
      ],
      content: "",
      autofocus: "end",
      onUpdate: () => { scheduleSave(); recomputeWordCount(); },
      editorProps: {
        handleClickOn(_view, _pos, node) {
          if (node.type.name === "permanoteLink" && node.attrs.id) {
            openPermanote(node.attrs.id);
            return true;
          }
          return false;
        },
      },
    });

    await loadDay();
    refreshTodos();
    refreshPermanotes();
    refreshDays();

    unlistenExternal = await listen<{ date: string; content: string }>(
      "day-changed-externally",
      (e) => {
        const { date: d } = e.payload;
        // Always refresh sidebar lists so calendar dots / todos / permanotes
        // reflect the external change.
        refreshTodos();
        refreshPermanotes();
        refreshDays();
        if (d !== date) return;
        if (!dirty) {
          // Clean editor — adopt the new content silently.
          loadDay();
        } else {
          // Surface a banner; user picks.
          externalConflict = e.payload;
        }
      },
    );
  });

  onDestroy(() => {
    if (saveTimer) clearTimeout(saveTimer);
    window.removeEventListener("keydown", onKeydown);
    window.removeEventListener("keydown", onLinkPickerKey);
    window.removeEventListener("permanote:open-link-picker", openLinkPicker as EventListener);
    themeMql?.removeEventListener("change", applyTheme);
    unlistenExternal?.();
    editor?.destroy();
  });

  function resolveKeepMine() {
    if (!confirm("Overwrite the version on disk with the version in this editor? The on-disk changes will be lost.")) return;
    externalConflict = null;
    // Force a save so our version wins.
    scheduleSave();
  }

  async function resolveUseTheirs() {
    externalConflict = null;
    await loadDay();
  }

  const headline = $derived.by(() => {
    if (!date) return "";
    const [y, m, d] = date.split("-").map(Number);
    const dt = new Date(y, m - 1, d);
    return dt.toLocaleDateString(undefined, {
      weekday: "long",
      month: "long",
      day: "numeric",
    });
  });

  const visibleTodos = $derived.by(() => {
    if (todoFilter === "open") return todos.filter((t) => !t.done);
    if (todoFilter === "done") return todos.filter((t) => t.done);
    if (todoFilter === "scheduled") return todos.filter((t) => !t.done && !!t.due);
    return todos;
  });

  const todosByDay = $derived.by(() => {
    const groups = new Map<string, Todo[]>();
    for (const t of visibleTodos) {
      if (!groups.has(t.day)) groups.set(t.day, []);
      groups.get(t.day)!.push(t);
    }
    return [...groups.entries()].sort((a, b) => b[0].localeCompare(a[0]));
  });

  function dayLabel(d: string): string {
    if (d === date) return "Today";
    const [y, m, dd] = d.split("-").map(Number);
    const dt = new Date(y, m - 1, dd);
    return dt.toLocaleDateString(undefined, {
      month: "short",
      day: "numeric",
    });
  }

  function escapeHtml(s: string): string {
    return s
      .replaceAll("&", "&amp;")
      .replaceAll("<", "&lt;")
      .replaceAll(">", "&gt;");
  }

  type CalCell = {
    date: string;
    day: number;
    inMonth: boolean;
    hasContent: boolean;
    hasOpen: boolean;
  };

  const dayMap = $derived.by(() => {
    const m = new Map<string, DayInfo>();
    for (const d of days) m.set(d.date, d);
    return m;
  });

  const calendarGrid = $derived.by<CalCell[]>(() => {
    if (!calendarMonth) return [];
    const [y, m] = calendarMonth.split("-").map(Number);
    const first = new Date(y, m - 1, 1);
    // Sunday-start grid. JS getDay(): 0=Sunday.
    const startWeekday = first.getDay();
    const gridStart = new Date(y, m - 1, 1 - startWeekday);
    const cells: CalCell[] = [];
    for (let i = 0; i < 42; i++) {
      const dt = new Date(gridStart.getFullYear(), gridStart.getMonth(), gridStart.getDate() + i);
      const ds = `${dt.getFullYear()}-${String(dt.getMonth() + 1).padStart(2, "0")}-${String(dt.getDate()).padStart(2, "0")}`;
      const info = dayMap.get(ds);
      cells.push({
        date: ds,
        day: dt.getDate(),
        inMonth: dt.getMonth() === m - 1,
        hasContent: !!info,
        hasOpen: !!info?.has_open_todos,
      });
    }
    return cells;
  });

  const calendarLabel = $derived.by(() => {
    if (!calendarMonth) return "";
    const [y, m] = calendarMonth.split("-").map(Number);
    return new Date(y, m - 1, 1).toLocaleDateString(undefined, {
      month: "long",
      year: "numeric",
    });
  });
</script>

<div class="shell">
<div class="app" class:focus={focusMode}>
  <aside class="panel panel-calendar">
    <div class="panel-head cal-head">
      <button class="cal-nav" onclick={() => shiftMonth(-1)} aria-label="Previous month">‹</button>
      <span class="cal-label">{calendarLabel}</span>
      <button class="cal-nav" onclick={() => shiftMonth(1)} aria-label="Next month">›</button>
    </div>
    <div class="cal-search">
      <input
        type="text"
        placeholder="Search…"
        bind:this={searchInputEl}
        bind:value={searchQuery}
        oninput={onSearchInput}
        onkeydown={(e) => {
          if (e.key === "Escape") {
            e.preventDefault();
            clearSearch();
            searchInputEl?.blur();
          }
        }}
      />
      {#if searchHits.length > 0}
        <div class="search-results">
          {#each searchHits as h (h.date)}
            <button
              class="search-hit"
              onclick={() => { jumpToDay(h.date); clearSearch(); }}
            >
              <span class="hit-date">{dayLabel(h.date)}</span>
              <span class="hit-snippet">{@html escapeHtml(h.snippet)
                .replaceAll('&lt;&lt;', '<mark>')
                .replaceAll('&gt;&gt;', '</mark>')}</span>
            </button>
          {/each}
        </div>
      {/if}
    </div>
    <div class="panel-body cal-body">
      <div class="cal-weekdays">
        <span>S</span><span>M</span><span>T</span><span>W</span><span>T</span><span>F</span><span>S</span>
      </div>
      <div class="cal-grid">
        {#each calendarGrid as cell (cell.date)}
          <button
            class="cal-cell"
            class:out={!cell.inMonth}
            class:has-content={cell.hasContent}
            class:has-open={cell.hasOpen}
            class:selected={cell.date === date}
            onclick={() => jumpToDay(cell.date)}
            title={cell.date}
          >
            <span class="cal-num">{cell.day}</span>
            <span class="cal-dots">
              {#if cell.hasOpen}<span class="cal-dot dot-open" title="Open todos"></span>{:else if cell.hasContent}<span class="cal-dot dot-content" title="Has content"></span>{/if}
            </span>
          </button>
        {/each}
      </div>
      <button class="cal-today" onclick={jumpToToday}>Today</button>
    </div>
    <div class="panel-foot">
      <button
        class="panel-foot-btn"
        title="Settings"
        aria-label="Open settings"
        onclick={() => (settingsOpen = true)}
      >
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <circle cx="12" cy="12" r="3"/>
          <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09a1.65 1.65 0 0 0-1-1.51 1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09a1.65 1.65 0 0 0 1.51-1 1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/>
        </svg>
      </button>
    </div>
  </aside>

  <main class="canvas">
    <div class="canvas-tools">
      <button
        class="canvas-tool"
        title={focusMode ? "Exit focus mode (Ctrl+\\)" : "Focus mode (Ctrl+\\)"}
        aria-pressed={focusMode}
        onclick={() => (focusMode = !focusMode)}
      >
        {#if focusMode}
          <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.3" aria-hidden="true">
            <path d="M6 2v3H3M10 2v3h3M6 14v-3H3M10 14v-3h3"/>
          </svg>
        {:else}
          <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.3" aria-hidden="true">
            <path d="M2 6V3h3M14 6V3h-3M2 10v3h3M14 10v3h-3"/>
          </svg>
        {/if}
      </button>
    </div>
    <div class="canvas-inner">
      <header>
      <h1>{headline}</h1>
    </header>

    {#if scheduledForToday.length}
      <aside class="scheduled">
        <div class="scheduled-head">Scheduled for today</div>
        {#each scheduledForToday as t (t.day + ':' + t.line)}
          <div class="scheduled-row">
            <button class="todo-mark" onclick={() => toggleTodo(t)} aria-label="Mark as done">□</button>
            <button class="scheduled-text" onclick={() => jumpToDay(t.day)} title="From {t.day}">{t.text}</button>
            <span class="scheduled-from">{dayLabel(t.day)}</span>
          </div>
        {/each}
      </aside>
    {/if}

    <div class="editor" bind:this={editorEl}></div>

    {#if externalConflict}
      <div class="conflict-banner" role="alert">
        <span class="conflict-text">
          This day was changed on another device.
        </span>
        <div class="conflict-actions">
          <button onclick={resolveKeepMine}>Keep mine</button>
          <button onclick={resolveUseTheirs}>Use theirs</button>
        </div>
      </div>
    {/if}

    <footer class="canvas-foot">
      <span class="date">{date}</span>
      <span class="word-count">{wordCount} {wordCount === 1 ? "word" : "words"}</span>
      <span class="status status-{status}">{status}</span>
    </footer>
    </div>

    {#if slashOpen && slashItemsList.length > 0}
      <div
        class="slash-menu"
        style="left: {slashLeft}px; top: {slashTop}px;"
        role="listbox"
      >
        {#each slashItemsList as item, i (item.title)}
          <button
            class="slash-item"
            class:active={i === slashIndex}
            onmousedown={(e) => { e.preventDefault(); pickSlash(i); }}
            onmouseenter={() => (slashIndex = i)}
            type="button"
          >
            <span class="slash-title">{item.title}</span>
            {#if item.hint}<span class="slash-hint">{item.hint}</span>{/if}
          </button>
        {/each}
      </div>
    {/if}

  </main>

  <aside class="panel panel-right">
    <section class="panel-section">
      <div class="panel-head">
        <span>Todos</span>
        <div class="filter">
          <button class:active={todoFilter === "open"} onclick={() => (todoFilter = "open")}>Open</button>
          <button class:active={todoFilter === "scheduled"} onclick={() => (todoFilter = "scheduled")}>Scheduled</button>
          <button class:active={todoFilter === "done"} onclick={() => (todoFilter = "done")}>Done</button>
          <button class:active={todoFilter === "all"} onclick={() => (todoFilter = "all")}>All</button>
        </div>
      </div>
      <div class="panel-body">
        {#if visibleTodos.length === 0}
          <div class="placeholder">No {todoFilter} todos.</div>
        {:else}
          {#each todosByDay as [day, items] (day)}
            <div class="todo-group">
              <div class="todo-day">{dayLabel(day)}</div>
              {#each items as t (day + ":" + t.line)}
                <div class="todo" class:done={t.done}>
                  <button
                    class="todo-mark"
                    onclick={() => toggleTodo(t)}
                    aria-label={t.done ? "Mark as open" : "Mark as done"}
                  >{t.done ? "■" : "□"}</button>
                  <button
                    class="todo-text"
                    onclick={() => jumpToDay(t.day)}
                    title="Jump to {t.day}"
                  >{t.text}</button>
                  {#if t.due}
                    <button
                      class="todo-due"
                      onclick={(e) => { e.stopPropagation(); openSchedule(t); }}
                      title="Scheduled {t.due}"
                    >{dueLabel(t.due)}</button>
                  {:else if !t.done}
                    <button
                      class="todo-sched"
                      onclick={(e) => { e.stopPropagation(); openSchedule(t); }}
                      aria-label="Schedule"
                      title="Schedule"
                    >📅</button>
                  {/if}
                </div>
                {#if scheduleOpenKey === day + ":" + t.line}
                  <div class="schedule-pop" role="dialog" aria-label="Schedule todo">
                    <input
                      type="date"
                      bind:value={scheduleDraft}
                      onkeydown={(e) => { if (e.key === "Enter" && scheduleDraft) setTodoDue(t, scheduleDraft); if (e.key === "Escape") scheduleOpenKey = null; }}
                    />
                    <div class="schedule-row">
                      <button onclick={() => setTodoDue(t, todayIso())}>Today</button>
                      <button onclick={() => setTodoDue(t, tomorrowIso())}>Tomorrow</button>
                      <button onclick={() => scheduleDraft && setTodoDue(t, scheduleDraft)} disabled={!scheduleDraft}>Set</button>
                      {#if t.due}
                        <button class="schedule-clear" onclick={() => setTodoDue(t, null)}>Clear</button>
                      {/if}
                    </div>
                  </div>
                {/if}
              {/each}
            </div>
          {/each}
        {/if}
      </div>
    </section>
    <section class="panel-section">
      <div class="panel-head">
        <span>Permanotes</span>
        <div class="panel-head-actions">
          <button
            class="perma-sort"
            class:active={permaFilterOpen || permaColorFilter !== "all" || permaQuery.trim().length > 0}
            onclick={() => (permaFilterOpen = !permaFilterOpen)}
            title="Filter"
            aria-label="Toggle filters"
            aria-pressed={permaFilterOpen}
          >
            <svg width="11" height="11" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
              <path d="M2 3h12M4 8h8M6 13h4"/>
            </svg>
          </button>
          <button
            class="perma-sort"
            onclick={() => (permaSort = permaSort === "recent" ? "title" : permaSort === "title" ? "manual" : "recent")}
            title="Toggle sort"
          >{permaSort === "recent" ? "↓ date" : permaSort === "title" ? "A–Z" : "manual"}</button>
        </div>
      </div>
      {#if permaFilterOpen}
      <div class="perma-filter">
        <input
          class="perma-search"
          type="text"
          placeholder="Filter…"
          bind:value={permaQuery}
        />
        <div class="perma-color-row">
          <button
            class="perma-color-chip"
            class:active={permaColorFilter === "all"}
            onclick={() => (permaColorFilter = "all")}
            title="All colors"
            aria-label="All colors"
          >·</button>
          {#each PERMA_COLORS as c (c)}
            <button
              class="perma-color-chip"
              data-color={c}
              class:active={permaColorFilter === c}
              onclick={() => (permaColorFilter = permaColorFilter === c ? "all" : c)}
              title={c}
              aria-label={`Filter to ${c}`}
            ></button>
          {/each}
        </div>
      </div>
      {/if}
      <div class="panel-body">
        {#if filteredPermanotes.length === 0}
          <div class="placeholder">
            {permanotes.length === 0 ? "No permanotes yet." : "No matches."}
          </div>
        {:else}
          {#each filteredPermanotes as p (p.day + ":" + p.id)}
            {@const collapsed = permaCollapsed.has(p.day + ':' + p.id)}
            <div
              class="perma"
              class:collapsed
              class:drag-over={dragOverId === p.id}
              class:dragging={dragId === p.id}
              data-color={p.color}
              role="button"
              tabindex="0"
              draggable="true"
              ondragstart={(e) => onPermaDragStart(e, p.id)}
              ondragover={(e) => onPermaDragOver(e, p.id)}
              ondragleave={() => onPermaDragLeave(p.id)}
              ondrop={(e) => onPermaDrop(e, p.id)}
              ondragend={onPermaDragEnd}
              onclick={() => openPermanote(p.id)}
              onkeydown={(e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); openPermanote(p.id); } }}
              title={p.title || "Untitled"}
            >
              <div class="perma-head">
                <button
                  class="perma-twirl"
                  aria-label={collapsed ? "Expand" : "Collapse"}
                  aria-expanded={!collapsed}
                  onclick={(e) => { e.stopPropagation(); togglePermaCollapsed(p); }}
                >
                  <svg width="8" height="8" viewBox="0 0 8 8" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="2 3 4 5 6 3"/></svg>
                </button>
                <span class="perma-title">{p.title || "Untitled"}</span>
                <span class="perma-day">{dayLabel(p.day)}</span>
              </div>
              {#if p.snippet && !collapsed}
                <div class="perma-snippet">{p.snippet}</div>
              {/if}
            </div>
          {/each}
        {/if}
      </div>
    </section>
  </aside>
</div>

{#if firstRun}
  <div class="onboard-backdrop" role="dialog" aria-modal="true" aria-label="Welcome to Permanote">
    <div class="onboard-card">
      <div class="onboard-eyebrow">Permanote</div>
      <h2 class="onboard-title">Choose your vault</h2>
      <p class="onboard-body">
        Permanote stores your daily notes as plain Markdown files on disk.
        Pick a folder to keep them in — somewhere synced (like OneDrive or Dropbox)
        works well. You can change this later in Settings.
      </p>
      <div class="onboard-path-row">
        <code class="onboard-path" title={onboardingPath}>{onboardingPath || "No folder selected"}</code>
        <button
          class="btn btn-secondary"
          onclick={async () => {
            const picked = await pickVaultFolder();
            if (picked) onboardingPath = picked;
          }}
        >Browse…</button>
      </div>
      <div class="onboard-actions">
        <button
          class="btn btn-primary"
          disabled={!onboardingPath}
          onclick={confirmOnboarding}
        >Use this folder</button>
      </div>
    </div>
  </div>
{/if}

{#if settingsOpen}
  <div
    class="settings-backdrop"
    onclick={() => (settingsOpen = false)}
    role="presentation"
  ></div>
  <div class="settings-panel" role="dialog" aria-modal="true" aria-label="Settings">
    <header class="settings-head">
      <h2>Settings</h2>
      <button
        class="settings-close"
        aria-label="Close settings"
        onclick={() => (settingsOpen = false)}
      >×</button>
    </header>
    <div class="settings-body">
      <section class="settings-section">
        <h3>Vault</h3>
        <div class="settings-row">
          <div class="settings-label">
            <span class="settings-label-main">Current folder</span>
            <code class="settings-path" title={vaultPath}>{vaultPath || "—"}</code>
          </div>
        </div>
        <div class="settings-row settings-row-actions">
          <button class="btn btn-secondary" onclick={changeVault}>Change vault…</button>
          <button class="btn btn-secondary" onclick={() => invoke('open_vault_folder').catch(console.error)}>Open vault folder</button>
        </div>
        <p class="settings-hint">Changing the vault relaunches Permanote.</p>
      </section>

      <section class="settings-section">
        <h3>Permanotes</h3>
        <div class="settings-row">
          <label class="settings-radio">
            <input
              type="radio"
              name="perma-mode"
              value="color"
              checked={settings.permanote_mode === "color"}
              onchange={() => saveSettings({ permanote_mode: "color" })}
            />
            <span>
              <span class="settings-label-main">Color mode</span>
              <span class="settings-label-sub">Tag permanotes by color. Filter and group by color.</span>
            </span>
          </label>
          <label class="settings-radio">
            <input
              type="radio"
              name="perma-mode"
              value="label"
              checked={settings.permanote_mode === "label"}
              onchange={() => saveSettings({ permanote_mode: "label" })}
            />
            <span>
              <span class="settings-label-main">Label mode <em class="badge">coming soon</em></span>
              <span class="settings-label-sub">Tag permanotes with your own labels and sort by them.</span>
            </span>
          </label>
        </div>
      </section>

      <section class="settings-section">
        <h3>Appearance</h3>
        <div class="settings-row">
          {#each ["system", "dark"] as t (t)}
            <label class="settings-radio settings-radio-inline">
              <input
                type="radio"
                name="theme"
                value={t}
                checked={settings.theme === t}
                onchange={() => saveSettings({ theme: t as Settings["theme"] })}
              />
              <span class="settings-label-main">{t.charAt(0).toUpperCase() + t.slice(1)}</span>
            </label>
          {/each}
        </div>
      </section>
    </div>
  </div>
{/if}

{#if shortcutsOpen}
  <div
    class="settings-backdrop"
    onclick={() => (shortcutsOpen = false)}
    role="presentation"
  ></div>
  <div class="shortcuts-panel" role="dialog" aria-modal="true" aria-label="Keyboard shortcuts">
    <header class="settings-head">
      <h2>Keyboard shortcuts</h2>
      <button
        class="settings-close"
        aria-label="Close shortcuts"
        onclick={() => (shortcutsOpen = false)}
      >×</button>
    </header>
    <div class="settings-body">
      <dl class="shortcuts-list">
        <dt><kbd>?</kbd></dt><dd>Show / hide this cheatsheet</dd>
        <dt><kbd>Ctrl</kbd>+<kbd>S</kbd></dt><dd>Save now</dd>
        <dt><kbd>Ctrl</kbd>+<kbd>K</kbd></dt><dd>Focus search</dd>
        <dt><kbd>Ctrl</kbd>+<kbd>\</kbd></dt><dd>Toggle focus mode</dd>
        <dt><kbd>←</kbd> <kbd>→</kbd></dt><dd>Previous / next day</dd>
        <dt><kbd>T</kbd></dt><dd>Jump to today</dd>
        <dt><kbd>Esc</kbd></dt><dd>Close this dialog</dd>
      </dl>
      <p class="settings-hint">Editor and input shortcuts (<kbd>Ctrl</kbd>+<kbd>S</kbd>, <kbd>Ctrl</kbd>+<kbd>\</kbd>, <kbd>Ctrl</kbd>+<kbd>K</kbd>) work everywhere; navigation shortcuts (<kbd>←</kbd> <kbd>→</kbd> <kbd>T</kbd> <kbd>?</kbd>) only outside text fields.</p>
    </div>
  </div>
{/if}

{#if permaDetail && permaDetailDraft}
  <div
    class="settings-backdrop"
    onclick={closePermanote}
    role="presentation"
  ></div>
  <div class="perma-detail-panel" role="dialog" aria-modal="true" aria-label="Permanote">
    <header class="settings-head">
      <div class="perma-detail-head-meta">
        <span class="perma-detail-eyebrow" data-color={permaDetailDraft.color}>Permanote</span>
        <span class="perma-detail-source">
          from <button class="perma-detail-source-link" onclick={() => { const d = permaDetail!.source_day; closePermanote(); if (d) jumpToDay(d); }}>{permaDetail.source_day ? dayLabel(permaDetail.source_day) : "unknown"}</button>
        </span>
      </div>
      <button
        class="settings-close"
        aria-label="Close"
        onclick={closePermanote}
      >×</button>
    </header>
    <div class="settings-body perma-detail-body">
      <input
        class="perma-detail-title"
        type="text"
        placeholder="Untitled"
        bind:value={permaDetailDraft.title}
      />

      <div class="perma-detail-colors">
        {#each PERMA_COLORS as c (c)}
          <button
            class="perma-color-chip"
            data-color={c}
            class:active={permaDetailDraft.color === c}
            onclick={() => (permaDetailDraft!.color = c)}
            title={c}
            aria-label={`Color ${c}`}
          ></button>
        {/each}
      </div>

      <textarea
        class="perma-detail-content"
        placeholder="Write your permanote…"
        bind:value={permaDetailDraft.content}
      ></textarea>

      {#if permaDetailError}
        <div class="perma-detail-error">{permaDetailError}</div>
      {/if}

      <section class="perma-detail-section">
        <h3>Backlinks</h3>
        {#if permaDetailBacklinks.length === 0}
          <div class="placeholder">No other days link to this permanote yet.</div>
        {:else}
          <ul class="perma-detail-backlinks">
            {#each permaDetailBacklinks as d (d)}
              <li>
                <button
                  class="perma-detail-backlink"
                  onclick={() => { closePermanote(); jumpToDay(d); }}
                >{dayLabel(d)}</button>
              </li>
            {/each}
          </ul>
        {/if}
      </section>

      <div class="perma-detail-meta">
        <span>Created {permaDetail.created ? new Date(permaDetail.created).toLocaleString() : "—"}</span>
        <span>Modified {permaDetail.modified ? new Date(permaDetail.modified).toLocaleString() : "—"}</span>
      </div>
    </div>
    <footer class="perma-detail-foot">
      <button
        class="btn btn-danger"
        disabled={permaDetailDeleting}
        onclick={deletePermanote}
      >{permaDetailDeleting ? "Deleting…" : "Delete"}</button>
      <div class="perma-detail-foot-right">
        <button class="btn btn-secondary" onclick={closePermanote}>Close</button>
        <button
          class="btn btn-primary"
          disabled={!permaDetailDirty || permaDetailSaving}
          onclick={savePermanote}
        >{permaDetailSaving ? "Saving…" : "Save"}</button>
      </div>
    </footer>
  </div>
{/if}

{#if linkPickerOpen}
  <div
    class="settings-backdrop"
    onclick={closeLinkPicker}
    role="presentation"
  ></div>
  <div class="link-picker" role="dialog" aria-modal="true" aria-label="Link permanote">
    <input
      class="link-picker-input"
      type="text"
      placeholder="Search permanotes…"
      bind:value={linkPickerQuery}
      oninput={() => (linkPickerIndex = 0)}
      use:focusOnMount
    />
    <div class="link-picker-list">
      {#if linkPickerHits.length === 0}
        <div class="placeholder">No matches.</div>
      {:else}
        {#each linkPickerHits as p, i (p.id)}
          <button
            class="link-picker-item"
            class:active={i === linkPickerIndex}
            data-color={p.color}
            onmouseenter={() => (linkPickerIndex = i)}
            onclick={() => insertLinkFromPicker(p)}
          >
            <span class="link-picker-dot" data-color={p.color}></span>
            <span class="link-picker-title">{p.title || "Untitled"}</span>
            <span class="link-picker-day">{dayLabel(p.day)}</span>
          </button>
        {/each}
      {/if}
    </div>
    <div class="link-picker-hint">↑↓ navigate · Enter insert · Esc cancel</div>
  </div>
{/if}

{#if toasts.length}
  <div class="toast-stack" aria-live="polite">
    {#each toasts as t (t.id)}
      <button class="toast toast-{t.kind}" onclick={() => dismissToast(t.id)} title="Dismiss">
        {t.message}
      </button>
    {/each}
  </div>
{/if}
</div>

<style>
  :global(:root) {
    color-scheme: dark;
    --bg: #0e0e0e;
    --fg: #d6d6d6;
    --fg-dim: #8a8a8a;
    --fg-faint: #555;
    --panel: #0a0a0a;
    --panel-2: #131313;
    --input-bg: #050505;
    --hover-bg: #161616;
    --accent-bg: #1d1d1d;
    --border: #1a1a1a;
    --border-2: #2a2a2a;
    --border-3: #444;
    font-family: "IBM Plex Sans", "Inter", system-ui, sans-serif;
  }

  :global(body) {
    margin: 0;
    background: var(--bg);
    color: var(--fg);
    overflow: hidden;
  }

  .shell {
    height: 100vh;
    width: 100vw;
  }

  .app {
    display: grid;
    grid-template-columns: 240px 1fr 280px;
    height: 100%;
    width: 100%;
    min-height: 0;
    transition: grid-template-columns 220ms ease;
  }
  .app.focus {
    grid-template-columns: 0 1fr 0;
  }
  .app.focus .panel {
    overflow: hidden;
    border-color: transparent;
    pointer-events: none;
    opacity: 0;
    transition: opacity 160ms ease;
  }

  .panel {
    background: var(--panel);
    border-color: var(--border);
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  .panel-foot {
    margin-top: auto;
    padding: 0.5rem 0.75rem 0.75rem;
    border-top: 1px solid var(--border);
    display: flex;
    align-items: center;
    justify-content: flex-start;
  }
  .panel-foot-btn {
    width: 28px;
    height: 28px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    background: transparent;
    border: 1px solid transparent;
    border-radius: 4px;
    color: var(--fg-faint);
    cursor: pointer;
    transition: color 120ms ease, border-color 120ms ease, background 120ms ease;
  }
  .panel-foot-btn:hover {
    color: var(--fg);
    border-color: var(--border-2);
    background: var(--hover-bg);
  }

  .panel-calendar {
    border-right: 1px solid var(--border);
  }

  .cal-head {
    padding: 1rem 1rem 0.75rem;
  }
  .cal-label {
    flex: 1;
    text-align: center;
    color: var(--fg);
    text-transform: none;
    letter-spacing: 0;
    font-family: "IBM Plex Sans", system-ui, sans-serif;
    font-size: 0.85rem;
    font-weight: 500;
  }
  .cal-nav {
    background: transparent;
    border: 0;
    color: var(--fg-dim);
    cursor: pointer;
    padding: 0 0.4rem;
    font-size: 1rem;
    line-height: 1;
  }
  .cal-nav:hover {
    color: var(--fg);
  }
  .cal-body {
    flex: 1 1 auto;
    min-height: 0;
    padding: 0 0.5rem 0.75rem;
    overflow-y: auto;
  }
  .cal-weekdays {
    display: grid;
    grid-template-columns: repeat(7, 1fr);
    text-align: center;
    font-family: "IBM Plex Mono", ui-monospace, monospace;
    font-size: 0.6rem;
    color: var(--fg-faint);
    text-transform: uppercase;
    letter-spacing: 0.1em;
    padding: 0.25rem 0;
  }
  .cal-grid {
    display: grid;
    grid-template-columns: repeat(7, 1fr);
    gap: 1px;
  }
  .cal-cell {
    aspect-ratio: 1 / 1;
    background: transparent;
    border: 1px solid transparent;
    color: var(--fg-dim);
    font: inherit;
    font-size: 0.75rem;
    cursor: pointer;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: 0;
    border-radius: 2px;
    position: relative;
  }
  .cal-cell:hover {
    background: var(--panel-2);
  }
  .cal-cell.out .cal-num {
    color: var(--fg-faint);
  }
  .cal-cell.selected {
    background: var(--fg);
    border-color: var(--fg);
    color: var(--panel);
  }
  .cal-cell.selected .cal-dot {
    background: var(--panel);
  }
  .cal-num {
    line-height: 1;
  }
  .cal-dots {
    display: flex;
    gap: 2px;
    margin-top: 3px;
    height: 3px;
  }
  .cal-dot {
    width: 3px;
    height: 3px;
    border-radius: 50%;
    background: var(--fg-faint);
  }
  .cal-dot.dot-content {
    background: var(--fg-dim);
  }
  .cal-dot.dot-open {
    background: #c08a3e;
  }
  .cal-today {
    margin-top: 0.75rem;
    width: 100%;
    background: transparent;
    border: 1px solid var(--border);
    color: var(--fg-dim);
    padding: 0.4rem;
    font: inherit;
    font-family: "IBM Plex Mono", ui-monospace, monospace;
    font-size: 0.65rem;
    text-transform: uppercase;
    letter-spacing: 0.1em;
    cursor: pointer;
    border-radius: 2px;
  }
  .cal-today:hover {
    border-color: var(--border-2);
    color: var(--fg);
  }

  .panel-right {
    border-left: 1px solid var(--border);
  }

  .panel-section {
    display: flex;
    flex-direction: column;
    flex: 1 1 0;
    min-height: 0;
    overflow: hidden;
  }

  .panel-section + .panel-section {
    border-top: 1px solid var(--border);
  }

  .panel-head {
    padding: 1rem 1.25rem 0.5rem;
    font-family: "IBM Plex Mono", ui-monospace, monospace;
    font-size: 0.65rem;
    color: var(--fg-dim);
    text-transform: uppercase;
    letter-spacing: 0.12em;
    flex: 0 0 auto;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
  }

  .filter {
    display: flex;
    gap: 0;
  }

  .filter button {
    background: none;
    border: 1px solid var(--border);
    color: var(--fg-faint);
    font-family: inherit;
    font-size: 0.6rem;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    padding: 0.2rem 0.4rem;
    cursor: pointer;
  }

  .filter button + button {
    border-left: none;
  }

  .filter button.active {
    color: var(--fg);
    background: var(--border);
  }

  .todo-group + .todo-group {
    margin-top: 1rem;
  }

  .todo-day {
    font-family: "IBM Plex Mono", ui-monospace, monospace;
    font-size: 0.6rem;
    color: var(--fg-faint);
    text-transform: uppercase;
    letter-spacing: 0.1em;
    margin-bottom: 0.4rem;
  }

  .todo {
    display: flex;
    gap: 0.5rem;
    align-items: flex-start;
    font-size: 0.85rem;
    line-height: 1.4;
    padding: 0.2rem 0;
    color: var(--fg);
  }

  .todo .todo-sched {
    opacity: 0;
    transition: opacity 120ms ease;
  }
  .todo:hover .todo-sched,
  .todo:focus-within .todo-sched {
    opacity: 1;
  }
  .todo-sched,
  .todo-due {
    flex: 0 0 auto;
    background: none;
    border: 0;
    padding: 0 4px;
    font: inherit;
    font-size: 0.65rem;
    line-height: 1.4;
    color: var(--fg-faint);
    cursor: pointer;
    font-family: "IBM Plex Mono", ui-monospace, monospace;
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }
  .todo-due {
    border: 1px solid var(--border);
    border-radius: 2px;
    padding: 0 5px;
    color: var(--fg-dim);
  }
  .todo-sched:hover,
  .todo-due:hover {
    color: var(--fg);
    border-color: var(--border-2);
  }

  .schedule-pop {
    margin: 4px 0 8px 1.4rem;
    padding: 8px;
    background: var(--panel-2);
    border: 1px solid var(--border);
    border-radius: 3px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .schedule-pop input[type="date"] {
    background: var(--panel);
    border: 1px solid var(--border);
    color: var(--fg);
    font: inherit;
    font-size: 0.75rem;
    padding: 3px 5px;
    border-radius: 2px;
    color-scheme: dark;
  }
  :global(html[data-theme="light"]) .schedule-pop input[type="date"] {
    color-scheme: light;
  }
  .schedule-row {
    display: flex;
    gap: 4px;
    flex-wrap: wrap;
  }
  .schedule-row button {
    background: transparent;
    border: 1px solid var(--border);
    color: var(--fg-dim);
    font: inherit;
    font-size: 0.65rem;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    padding: 2px 8px;
    border-radius: 2px;
    cursor: pointer;
    font-family: "IBM Plex Mono", ui-monospace, monospace;
  }
  .schedule-row button:hover:not(:disabled) {
    color: var(--fg);
    border-color: var(--border-2);
  }
  .schedule-row button:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .schedule-row .schedule-clear {
    margin-left: auto;
    color: var(--fg-faint);
  }

  .todo.done .todo-text {
    color: var(--fg-faint);
    text-decoration: line-through;
  }

  .todo-mark {
    color: var(--fg-dim);
    flex: 0 0 auto;
    font-size: 0.9rem;
    line-height: 1.3;
    background: none;
    border: none;
    padding: 0;
    cursor: pointer;
    font-family: inherit;
  }

  .todo-mark:hover {
    color: var(--fg);
  }

  .todo-text {
    flex: 1 1 auto;
    background: none;
    border: none;
    color: inherit;
    font: inherit;
    text-align: left;
    padding: 0;
    cursor: pointer;
  }

  .todo-text:hover {
    color: var(--fg);
    text-decoration: underline;
  }

  .perma-sort {
    background: transparent;
    border: 0;
    color: var(--fg-faint);
    font-family: "IBM Plex Mono", ui-monospace, monospace;
    font-size: 0.6rem;
    text-transform: uppercase;
    letter-spacing: 0.1em;
    cursor: pointer;
    padding: 2px 4px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }
  .perma-sort:hover {
    color: var(--fg);
  }
  .perma-sort.active {
    color: var(--fg);
  }
  .panel-head-actions {
    display: inline-flex;
    align-items: center;
    gap: 2px;
  }
  .perma-filter {
    padding: 0 0.85rem 0.5rem;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .perma-search {
    background: var(--panel);
    border: 1px solid var(--border);
    color: var(--fg);
    font: inherit;
    font-size: 0.7rem;
    padding: 4px 6px;
    border-radius: 2px;
    outline: none;
  }
  .perma-search:focus {
    border-color: var(--border-2);
  }
  .perma-color-row {
    display: flex;
    gap: 5px;
    align-items: center;
  }
  .perma-color-chip {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: var(--chip-color, var(--border-2));
    border: 1px solid transparent;
    padding: 0;
    cursor: pointer;
    opacity: 0.5;
    font-size: 8px;
    line-height: 1;
    color: var(--fg-faint);
  }
  .perma-color-chip:hover {
    opacity: 1;
  }
  .perma-color-chip.active {
    opacity: 1;
    border-color: rgba(255, 255, 255, 0.4);
  }
  .perma-color-chip[data-color="amber"]  { --chip-color: #c08a3e; }
  .perma-color-chip[data-color="cobalt"] { --chip-color: #4a78c0; }
  .perma-color-chip[data-color="rose"]   { --chip-color: #c04a78; }
  .perma-color-chip[data-color="sage"]   { --chip-color: #6a9874; }
  .perma-color-chip[data-color="violet"] { --chip-color: #8a4ac0; }
  .perma-color-chip[data-color="slate"]  { --chip-color: #6a7480; }

  .perma {
    display: block;
    width: 100%;
    text-align: left;
    background: var(--panel-2);
    border: 0;
    border-left: 3px solid var(--accent, #c08a3e);
    padding: 0.5rem 0.75rem;
    margin-bottom: 0.5rem;
    cursor: pointer;
    color: inherit;
    font: inherit;
    border-radius: 0 2px 2px 0;
    transition: background 120ms ease;
  }

  .perma[data-color="amber"]  { --accent: #c08a3e; }
  .perma[data-color="cobalt"] { --accent: #4a78c0; }
  .perma[data-color="rose"]   { --accent: #c04a78; }
  .perma[data-color="sage"]   { --accent: #6a9874; }
  .perma[data-color="violet"] { --accent: #8a4ac0; }
  .perma[data-color="slate"]  { --accent: #6a7480; }

  .perma:hover {
    background: var(--hover-bg);
  }
  .perma {
    cursor: grab;
  }
  .perma:active {
    cursor: grabbing;
  }
  .perma.dragging {
    opacity: 0.4;
  }
  .perma.drag-over {
    box-shadow: inset 0 2px 0 0 var(--accent, #c08a3e);
  }
  .perma-twirl {
    background: transparent;
    border: 0;
    padding: 2px;
    margin-right: 2px;
    color: var(--fg-faint);
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    transition: transform 120ms ease, color 120ms ease;
  }
  .perma-twirl:hover {
    color: var(--fg-dim);
  }
  .perma.collapsed .perma-twirl {
    transform: rotate(-90deg);
  }

  .perma-head {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: 0.5rem;
    margin-bottom: 0.25rem;
    min-width: 0;
  }

  .perma-title {
    font-size: 0.85rem;
    font-weight: 500;
    color: var(--accent);
    min-width: 0;
    overflow-wrap: anywhere;
  }

  .perma-day {
    font-family: "IBM Plex Mono", ui-monospace, monospace;
    font-size: 0.6rem;
    color: var(--fg-faint);
    text-transform: uppercase;
    letter-spacing: 0.08em;
    flex: 0 0 auto;
  }

  .perma-snippet {
    font-size: 0.75rem;
    color: var(--fg-dim);
    line-height: 1.4;
    overflow: hidden;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
  }

  .panel-body {
    flex: 1 1 auto;
    overflow-y: auto;
    overflow-x: hidden;
    padding: 0.5rem 1.25rem 1.25rem;
    overflow-wrap: anywhere;
    word-break: break-word;
    min-width: 0;
  }

  .placeholder {
    color: var(--fg-faint);
    font-size: 0.8rem;
    font-style: italic;
  }

  .canvas {
    width: 100%;
    height: 100vh;
    overflow-y: auto;
    box-sizing: border-box;
    position: relative;
  }
  .canvas-inner {
    max-width: 760px;
    width: 100%;
    min-height: 100%;
    margin: 0 auto;
    padding: 4rem 2rem 0.75rem;
    box-sizing: border-box;
    display: flex;
    flex-direction: column;
  }
  .canvas-tools {
    position: absolute;
    top: 1rem;
    right: 1rem;
    display: flex;
    gap: 0.4rem;
    z-index: 5;
  }
  .canvas-tool {
    width: 28px;
    height: 28px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    background: transparent;
    border: 1px solid transparent;
    border-radius: 4px;
    color: var(--fg-faint);
    cursor: pointer;
    transition: color 120ms ease, border-color 120ms ease, background 120ms ease;
  }
  .canvas-tool:hover {
    color: var(--fg);
    border-color: var(--border-2);
    background: var(--hover-bg);
  }
  .canvas-tool[aria-pressed="true"] {
    color: var(--fg);
    border-color: var(--border-2);
  }

  header {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    margin-bottom: 1.5rem;
  }

  h1 {
    font-size: 1.5rem;
    font-weight: 600;
    margin: 0;
    letter-spacing: -0.01em;
  }

  .canvas-foot {
    margin-top: auto;
    padding-top: 0.75rem;
    border-top: 1px solid var(--border);
    min-height: 28px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
    font-family: "IBM Plex Mono", ui-monospace, monospace;
    font-size: 0.65rem;
    color: var(--fg-faint);
    text-transform: uppercase;
    letter-spacing: 0.08em;
  }

  .status-saving { color: var(--fg-dim); }
  .status-saved { color: var(--fg-faint); }
  .status-error { color: #ff5a5a; }
  .status-loading { color: var(--fg-dim); }

  .cal-search {
    position: relative;
    padding: 0.5rem 0.75rem 0.25rem;
  }
  .cal-search input {
    width: 100%;
    background: var(--panel-2);
    border: 1px solid var(--border);
    color: var(--fg);
    font: inherit;
    font-size: 0.75rem;
    padding: 0.3rem 0.5rem;
    border-radius: 2px;
    outline: none;
    box-sizing: border-box;
  }
  .cal-search input:focus {
    border-color: var(--border-2);
  }

  .search-results {
    position: absolute;
    top: calc(100% + 4px);
    left: 0.75rem;
    right: 0.75rem;
    max-height: 60vh;
    overflow-y: auto;
    background: var(--panel-2);
    border: 1px solid var(--border-2);
    border-radius: 2px;
    z-index: 50;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.5);
  }
  .search-hit {
    display: block;
    width: 100%;
    text-align: left;
    background: transparent;
    border: 0;
    border-bottom: 1px solid var(--border);
    color: var(--fg);
    padding: 0.6rem 0.75rem;
    cursor: pointer;
    font: inherit;
  }
  .search-hit:hover {
    background: var(--panel-2);
  }
  .search-hit:last-child {
    border-bottom: 0;
  }
  .hit-date {
    display: block;
    font-family: "IBM Plex Mono", ui-monospace, monospace;
    font-size: 0.65rem;
    color: var(--fg-dim);
    text-transform: uppercase;
    letter-spacing: 0.08em;
    margin-bottom: 0.2rem;
  }
  .hit-snippet {
    display: block;
    font-size: 0.8rem;
    line-height: 1.45;
    color: var(--fg);
    overflow: hidden;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
  }
  :global(.hit-snippet mark) {
    background: #c08a3e;
    color: var(--panel);
    padding: 0 2px;
    border-radius: 1px;
  }

  .editor {
    flex: 1;
    font-size: 1rem;
    line-height: 1.7;
  }

  :global(.editor .tiptap) {
    outline: none;
    min-height: 60vh;
  }

  :global(.editor p) {
    margin: 0 0 0.75rem;
  }

  :global(.editor h1, .editor h2, .editor h3) {
    font-weight: 600;
    margin: 1.5rem 0 0.75rem;
    letter-spacing: -0.01em;
  }

  :global(.editor h1) { font-size: 1.5rem; }
  :global(.editor h2) { font-size: 1.25rem; }
  :global(.editor h3) { font-size: 1.05rem; }

  :global(.editor ul, .editor ol) {
    padding-left: 1.5rem;
    margin: 0 0 0.75rem;
  }

  :global(.editor ul[data-type="taskList"]) {
    list-style: none;
    padding-left: 0;
  }

  :global(.editor ul[data-type="taskList"] li) {
    display: flex;
    align-items: flex-start;
    gap: 0.5rem;
    margin: 0 0 0.25rem;
  }

  :global(.editor ul[data-type="taskList"] li > label) {
    flex: 0 0 auto;
    margin-top: 0.15em;
    line-height: 1;
    user-select: none;
  }

  :global(.editor ul[data-type="taskList"] li > label input[type="checkbox"]) {
    transform: translateY(0.05em);
    width: 0.95em;
    height: 0.95em;
    margin: 0;
  }

  :global(.editor ul[data-type="taskList"] li > div) {
    flex: 1 1 auto;
  }

  :global(.editor ul[data-type="taskList"] li input[type="checkbox"]) {
    accent-color: var(--fg);
    cursor: pointer;
  }

  :global(.editor ul[data-type="taskList"] li[data-checked="true"] > div) {
    color: var(--fg-dim);
    text-decoration: line-through;
  }

  :global(.editor blockquote) {
    border-left: 2px solid var(--border-2);
    padding-left: 1rem;
    color: var(--fg-dim);
    margin: 0 0 0.75rem;
  }

  :global(.editor .permanote) {
    border: 1px solid var(--border-2);
    border-left: 3px solid var(--accent, #c08a3e);
    border-radius: 2px;
    padding: 0.75rem 1rem;
    margin: 1rem 0;
    background: var(--panel-2);
  }

  :global(.editor .permanote[data-color="amber"]) { --accent: #c08a3e; }
  :global(.editor .permanote[data-color="cobalt"]) { --accent: #4a78c0; }
  :global(.editor .permanote[data-color="rose"])   { --accent: #c04a78; }
  :global(.editor .permanote[data-color="sage"])   { --accent: #6a9874; }
  :global(.editor .permanote[data-color="violet"]) { --accent: #8a4ac0; }
  :global(.editor .permanote[data-color="slate"])  { --accent: #6a7480; }

  :global(.editor .permanote-head) {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.75rem;
    font-family: "IBM Plex Mono", ui-monospace, monospace;
    font-size: 0.65rem;
    color: var(--accent);
    text-transform: uppercase;
    letter-spacing: 0.1em;
    margin-bottom: 0.5rem;
    user-select: none;
  }

  :global(.editor .permanote-title) {
    flex: 1;
    min-width: 0;
    color: var(--accent);
    outline: none;
    cursor: text;
    user-select: text;
    -webkit-user-select: text;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  :global(.editor .permanote-title:empty::before) {
    content: attr(data-placeholder);
    color: var(--fg-faint);
  }
  :global(.editor .permanote-title:focus) {
    color: var(--fg);
  }

  :global(.editor .permanote-swatch-wrap) {
    position: relative;
    flex: 0 0 auto;
    display: flex;
    align-items: center;
  }
  :global(.editor .permanote-swatch) {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: var(--accent);
    border: 1px solid rgba(255, 255, 255, 0.15);
    padding: 0;
    cursor: pointer;
    opacity: 0.7;
    transition: opacity 0.1s ease, transform 0.1s ease;
  }
  :global(.editor .permanote-swatch:hover) {
    opacity: 1;
    transform: scale(1.15);
  }
  :global(.editor .permanote-color-popover) {
    position: absolute;
    top: calc(100% + 6px);
    right: 0;
    z-index: 50;
    display: flex;
    gap: 6px;
    padding: 6px 8px;
    background: var(--panel-2);
    border: 1px solid var(--border-2);
    border-radius: 4px;
    box-shadow: 0 6px 18px rgba(0, 0, 0, 0.4);
  }
  :global(.editor .permanote-color-popover[hidden]) {
    display: none;
  }
  :global(.editor .permanote-dot) {
    width: 12px;
    height: 12px;
    border-radius: 50%;
    background: var(--dot-color, var(--border-3));
    border: 1px solid transparent;
    padding: 0;
    cursor: pointer;
    opacity: 0.85;
    transition: opacity 0.1s ease, transform 0.1s ease;
  }
  :global(.editor .permanote-dot:hover) {
    opacity: 1;
    transform: scale(1.15);
  }
  :global(.editor .permanote-dot[data-color="amber"])  { --dot-color: #c08a3e; }
  :global(.editor .permanote-dot[data-color="cobalt"]) { --dot-color: #4a78c0; }
  :global(.editor .permanote-dot[data-color="rose"])   { --dot-color: #c04a78; }
  :global(.editor .permanote-dot[data-color="sage"])   { --dot-color: #6a9874; }
  :global(.editor .permanote-dot[data-color="violet"]) { --dot-color: #8a4ac0; }
  :global(.editor .permanote-dot[data-color="slate"])  { --dot-color: #6a7480; }
  :global(.editor .permanote[data-color="amber"]  .permanote-color-popover .permanote-dot[data-color="amber"]),
  :global(.editor .permanote[data-color="cobalt"] .permanote-color-popover .permanote-dot[data-color="cobalt"]),
  :global(.editor .permanote[data-color="rose"]   .permanote-color-popover .permanote-dot[data-color="rose"]),
  :global(.editor .permanote[data-color="sage"]   .permanote-color-popover .permanote-dot[data-color="sage"]),
  :global(.editor .permanote[data-color="violet"] .permanote-color-popover .permanote-dot[data-color="violet"]),
  :global(.editor .permanote[data-color="slate"]  .permanote-color-popover .permanote-dot[data-color="slate"]) {
    border-color: rgba(255, 255, 255, 0.35);
  }

  :global(.editor .permanote-id) {
    display: none;
  }

  .conflict-banner {
    position: fixed;
    left: 50%;
    bottom: 2rem;
    transform: translateX(-50%);
    display: flex;
    align-items: center;
    gap: 1rem;
    padding: 8px 14px;
    background: #1a140a;
    border: 1px solid #c08a3e;
    border-radius: 3px;
    color: var(--fg);
    font-size: 0.72rem;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.5);
    z-index: 200;
  }
  .conflict-text {
    color: #d8b078;
  }
  .conflict-actions {
    display: flex;
    gap: 6px;
  }
  .conflict-actions button {
    background: transparent;
    border: 1px solid var(--border-2);
    color: var(--fg);
    font: inherit;
    font-size: 0.7rem;
    padding: 3px 10px;
    cursor: pointer;
    border-radius: 2px;
  }
  .conflict-actions button:hover {
    background: var(--accent-bg);
    border-color: var(--border-3);
  }

  .slash-menu {
    position: fixed;
    z-index: 1000;
    min-width: 220px;
    max-height: 280px;
    overflow-y: auto;
    background: var(--panel-2);
    border: 1px solid var(--border-2);
    border-radius: 4px;
    padding: 4px;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.4);
    font-family: "IBM Plex Mono", ui-monospace, monospace;
    font-size: 0.72rem;
  }
  .slash-item {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    width: 100%;
    background: transparent;
    border: 0;
    color: var(--fg);
    padding: 6px 10px;
    text-align: left;
    cursor: pointer;
    border-radius: 2px;
  }
  .slash-item.active {
    background: var(--accent-bg);
  }
  .slash-title {
    color: var(--fg);
  }
  .slash-hint {
    color: var(--fg-faint);
    font-size: 0.62rem;
    margin-left: 12px;
  }

  :global(.editor .permanote-body > *:last-child) {
    margin-bottom: 0;
  }

  :global(.editor code) {
    font-family: "IBM Plex Mono", ui-monospace, monospace;
    font-size: 0.9em;
    background: var(--border);
    padding: 0.1em 0.3em;
    border-radius: 2px;
  }

  .cal-foot {
    display: none;
  }

  /* ── Light theme overrides ─────────────────────────────────────────── */
  :global(html[data-theme="light"]) {
    --bg: #f6f6f4;
    --fg: #2a2a2a;
    --fg-dim: #6a6a6a;
    --fg-faint: #9a9a9a;
    --panel: #fbfbf9;
    --panel-2: #f0f0ec;
    --input-bg: #fafaf8;
    --hover-bg: #ececea;
    --accent-bg: #dededa;
    --border: #e4e4e0;
    --border-2: #c8c8c4;
    --border-3: #a8a8a4;
  }

  /* ── First-run onboarding ──────────────────────────────────────────── */
  .onboard-backdrop {
    position: fixed;
    inset: 0;
    background: var(--bg);
    z-index: 100;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 2rem;
  }
  .onboard-card {
    width: 100%;
    max-width: 480px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--panel);
    padding: 2.25rem 2rem 1.75rem;
    box-shadow: 0 20px 60px rgba(0, 0, 0, 0.5);
  }
  :global(html[data-theme="light"]) .onboard-card {
    box-shadow: 0 20px 60px rgba(0, 0, 0, 0.08);
  }
  .onboard-eyebrow {
    font-family: "IBM Plex Mono", ui-monospace, monospace;
    font-size: 0.62rem;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: var(--fg-faint);
    margin-bottom: 1rem;
  }
  .onboard-title {
    font-size: 1.4rem;
    font-weight: 600;
    margin: 0 0 0.75rem;
    color: var(--fg);
  }
  .onboard-body {
    font-size: 0.85rem;
    line-height: 1.55;
    color: var(--fg-dim);
    margin: 0 0 1.5rem;
  }
  .onboard-path-row {
    display: flex;
    align-items: stretch;
    gap: 0.5rem;
    margin-bottom: 1.5rem;
  }
  .onboard-path {
    flex: 1 1 auto;
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 0.55rem 0.7rem;
    background: var(--input-bg);
    font-family: "IBM Plex Mono", ui-monospace, monospace;
    font-size: 0.72rem;
    color: var(--fg);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .onboard-actions {
    display: flex;
    justify-content: flex-end;
  }

  /* ── Buttons (shared) ──────────────────────────────────────────────── */
  .btn {
    font-family: "IBM Plex Sans", ui-sans-serif, system-ui, sans-serif;
    font-size: 0.78rem;
    padding: 0.5rem 0.95rem;
    border-radius: 4px;
    cursor: pointer;
    transition: background 120ms ease, border-color 120ms ease, color 120ms ease;
    border: 1px solid transparent;
  }
  .btn:disabled { opacity: 0.4; cursor: not-allowed; }
  .btn-primary {
    background: var(--fg);
    color: var(--bg);
    border-color: var(--fg);
  }
  .btn-primary:not(:disabled):hover { opacity: 0.85; }
  .btn-secondary {
    background: transparent;
    color: var(--fg-dim);
    border-color: var(--border-2);
  }
  .btn-secondary:hover {
    color: var(--fg);
    border-color: var(--border-3);
    background: var(--hover-bg);
  }

  /* ── Settings panel ────────────────────────────────────────────────── */
  .settings-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.45);
    z-index: 80;
  }
  .settings-panel {
    position: fixed;
    top: 0;
    right: 0;
    bottom: 0;
    width: min(420px, 100vw);
    background: var(--panel);
    border-left: 1px solid var(--border);
    z-index: 90;
    display: flex;
    flex-direction: column;
    box-shadow: -20px 0 50px rgba(0, 0, 0, 0.4);
  }
  :global(html[data-theme="light"]) .settings-panel {
    box-shadow: -20px 0 50px rgba(0, 0, 0, 0.06);
  }
  .shortcuts-panel {
    position: fixed;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    width: min(440px, calc(100vw - 2rem));
    max-height: calc(100vh - 4rem);
    background: var(--panel);
    border: 1px solid var(--border);
    border-radius: 4px;
    z-index: 90;
    display: flex;
    flex-direction: column;
    box-shadow: 0 20px 60px rgba(0, 0, 0, 0.5);
  }
  :global(html[data-theme="light"]) .shortcuts-panel {
    box-shadow: 0 20px 60px rgba(0, 0, 0, 0.12);
  }
  .shortcuts-list {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 0.6rem 1.25rem;
    margin: 0;
    align-items: center;
  }
  .shortcuts-list dt {
    margin: 0;
    display: flex;
    align-items: center;
    gap: 4px;
    justify-self: end;
  }
  .shortcuts-list dd {
    margin: 0;
    font-size: 0.8rem;
    color: var(--fg-dim);
  }
  kbd {
    font-family: "IBM Plex Mono", ui-monospace, monospace;
    font-size: 0.7rem;
    background: var(--panel-2);
    border: 1px solid var(--border);
    border-bottom-width: 2px;
    border-radius: 3px;
    padding: 1px 6px;
    color: var(--fg);
    white-space: nowrap;
  }
  .settings-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 1.25rem 1.5rem;
    border-bottom: 1px solid var(--border);
  }
  .settings-head h2 {
    margin: 0;
    font-size: 0.95rem;
    font-weight: 600;
    color: var(--fg);
  }
  .settings-close {
    width: 28px;
    height: 28px;
    border-radius: 4px;
    background: transparent;
    border: 1px solid transparent;
    color: var(--fg-faint);
    font-size: 1.2rem;
    line-height: 1;
    cursor: pointer;
  }
  .settings-close:hover { color: var(--fg); border-color: var(--border-2); background: var(--hover-bg); }
  .settings-body {
    flex: 1 1 auto;
    overflow-y: auto;
    padding: 1.25rem 1.5rem 2rem;
  }
  .settings-section {
    padding: 1rem 0 1.5rem;
    border-bottom: 1px solid var(--border);
  }
  .settings-section:last-child { border-bottom: none; }
  .settings-section h3 {
    margin: 0 0 0.9rem;
    font-family: "IBM Plex Mono", ui-monospace, monospace;
    font-size: 0.62rem;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: var(--fg-faint);
    font-weight: 500;
  }
  .settings-row {
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
    margin-bottom: 0.75rem;
  }
  .settings-row-actions {
    flex-direction: row;
    flex-wrap: wrap;
  }
  .settings-label {
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
  }
  .settings-label-main {
    font-size: 0.82rem;
    color: var(--fg);
  }
  .settings-label-sub {
    font-size: 0.72rem;
    color: var(--fg-dim);
    display: block;
    margin-top: 0.15rem;
    line-height: 1.4;
  }
  .settings-path {
    font-family: "IBM Plex Mono", ui-monospace, monospace;
    font-size: 0.7rem;
    color: var(--fg-dim);
    background: var(--input-bg);
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 0.45rem 0.6rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    display: block;
  }
  .settings-radio {
    display: flex;
    gap: 0.65rem;
    cursor: pointer;
    padding: 0.4rem 0;
    align-items: flex-start;
  }
  .settings-radio input { margin-top: 0.25rem; accent-color: var(--fg); }
  .settings-radio-inline { display: inline-flex; align-items: center; margin-right: 1rem; }
  .settings-radio-inline input { margin-top: 0; }
  .badge {
    display: inline-block;
    font-style: normal;
    font-family: "IBM Plex Mono", ui-monospace, monospace;
    font-size: 0.55rem;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--fg-faint);
    border: 1px solid var(--border-2);
    border-radius: 3px;
    padding: 0.05rem 0.35rem;
    margin-left: 0.4rem;
    vertical-align: middle;
  }
  .settings-hint {
    font-size: 0.7rem;
    color: var(--fg-faint);
    margin: 0.5rem 0 0;
    line-height: 1.5;
  }

  .perma-detail-panel {
    position: fixed;
    top: 0;
    right: 0;
    bottom: 0;
    width: min(560px, 92vw);
    background: var(--panel);
    border-left: 1px solid var(--border-2);
    display: flex;
    flex-direction: column;
    z-index: 200;
    box-shadow: -16px 0 48px rgba(0, 0, 0, 0.5);
  }
  :global(html[data-theme="light"]) .perma-detail-panel {
    box-shadow: -16px 0 48px rgba(0, 0, 0, 0.12);
  }
  .perma-detail-head-meta {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
  }
  .perma-detail-eyebrow {
    font-size: 0.62rem;
    text-transform: uppercase;
    letter-spacing: 0.12em;
    color: var(--fg-faint);
    font-family: "IBM Plex Mono", ui-monospace, monospace;
  }
  .perma-detail-eyebrow[data-color="amber"]  { color: #c08a3e; }
  .perma-detail-eyebrow[data-color="cobalt"] { color: #4a78c0; }
  .perma-detail-eyebrow[data-color="rose"]   { color: #c04a78; }
  .perma-detail-eyebrow[data-color="sage"]   { color: #6a9874; }
  .perma-detail-eyebrow[data-color="violet"] { color: #8a4ac0; }
  .perma-detail-eyebrow[data-color="slate"]  { color: #6a7480; }
  .perma-detail-source {
    font-size: 0.72rem;
    color: var(--fg-dim);
  }
  .perma-detail-source-link {
    background: none;
    border: none;
    color: var(--fg);
    padding: 0;
    cursor: pointer;
    text-decoration: underline;
    text-decoration-color: var(--border-3);
    text-underline-offset: 3px;
    font: inherit;
  }
  .perma-detail-source-link:hover { text-decoration-color: var(--fg); }
  .perma-detail-body {
    flex: 1 1 auto;
    overflow-y: auto;
    padding: 1.25rem 1.5rem;
    display: flex;
    flex-direction: column;
    gap: 1rem;
  }
  .perma-detail-title {
    background: transparent;
    border: none;
    border-bottom: 1px solid var(--border-2);
    color: var(--fg);
    font-family: "IBM Plex Sans", system-ui, sans-serif;
    font-size: 1.4rem;
    font-weight: 600;
    padding: 0.25rem 0 0.5rem;
    outline: none;
  }
  .perma-detail-title::placeholder { color: var(--fg-faint); }
  .perma-detail-title:focus { border-bottom-color: var(--fg-dim); }
  .perma-detail-colors {
    display: flex;
    gap: 0.4rem;
  }
  .perma-detail-content {
    background: var(--input-bg);
    border: 1px solid var(--border-2);
    border-radius: 2px;
    color: var(--fg);
    font-family: "IBM Plex Mono", ui-monospace, monospace;
    font-size: 0.78rem;
    line-height: 1.55;
    padding: 0.75rem 0.85rem;
    min-height: 220px;
    resize: vertical;
    outline: none;
  }
  .perma-detail-content:focus { border-color: var(--border-3); }
  .perma-detail-error {
    color: #ff5a5a;
    font-size: 0.75rem;
    font-family: "IBM Plex Mono", ui-monospace, monospace;
  }
  .perma-detail-section {
    border-top: 1px solid var(--border);
    padding-top: 0.75rem;
  }
  .perma-detail-section h3 {
    font-size: 0.62rem;
    text-transform: uppercase;
    letter-spacing: 0.12em;
    color: var(--fg-dim);
    margin: 0 0 0.5rem;
    font-weight: 500;
  }
  .perma-detail-backlinks {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
  }
  .perma-detail-backlink {
    background: none;
    border: none;
    color: var(--fg);
    padding: 0.25rem 0;
    cursor: pointer;
    text-align: left;
    font: inherit;
    font-size: 0.78rem;
  }
  .perma-detail-backlink:hover { color: var(--fg-dim); }
  .perma-detail-meta {
    display: flex;
    justify-content: space-between;
    gap: 1rem;
    font-size: 0.68rem;
    color: var(--fg-faint);
    font-family: "IBM Plex Mono", ui-monospace, monospace;
    border-top: 1px solid var(--border);
    padding-top: 0.6rem;
  }
  .perma-detail-foot {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 0.5rem;
    padding: 0.85rem 1.5rem;
    border-top: 1px solid var(--border-2);
    background: var(--panel-2);
  }
  .perma-detail-foot-right {
    display: flex;
    gap: 0.5rem;
  }
  .btn-danger {
    background: transparent;
    border: 1px solid var(--border-2);
    color: #ff5a5a;
    padding: 0.4rem 0.85rem;
    font-size: 0.75rem;
    border-radius: 2px;
    cursor: pointer;
    font-family: inherit;
  }
  .btn-danger:not(:disabled):hover {
    border-color: #ff5a5a;
    background: rgba(255, 90, 90, 0.08);
  }
  .btn-danger:disabled { opacity: 0.5; cursor: default; }

  :global(.editor a.permanote-link) {
    display: inline-flex;
    align-items: baseline;
    gap: 0.2em;
    padding: 0.05em 0.4em;
    border: 1px solid var(--border-2);
    border-radius: 2px;
    background: var(--panel-2);
    color: var(--fg);
    text-decoration: none;
    font-size: 0.92em;
    cursor: pointer;
    line-height: 1.3;
  }
  :global(.editor a.permanote-link::before) {
    content: "¶";
    color: var(--fg-faint);
    font-size: 0.78em;
  }
  :global(.editor a.permanote-link:hover) {
    border-color: var(--border-3);
    background: var(--hover-bg);
  }
  :global(.editor a.permanote-link.ProseMirror-selectednode) {
    outline: 1px solid var(--fg-dim);
    outline-offset: 1px;
  }

  .link-picker {
    position: fixed;
    top: 18vh;
    left: 50%;
    transform: translateX(-50%);
    width: min(520px, 92vw);
    background: var(--panel);
    border: 1px solid var(--border-2);
    border-radius: 4px;
    z-index: 220;
    box-shadow: 0 24px 64px rgba(0, 0, 0, 0.55);
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  :global(html[data-theme="light"]) .link-picker {
    box-shadow: 0 24px 64px rgba(0, 0, 0, 0.12);
  }
  .link-picker-input {
    background: transparent;
    border: none;
    border-bottom: 1px solid var(--border);
    color: var(--fg);
    font-family: "IBM Plex Sans", system-ui, sans-serif;
    font-size: 0.95rem;
    padding: 0.75rem 1rem;
    outline: none;
  }
  .link-picker-input::placeholder { color: var(--fg-faint); }
  .link-picker-list {
    max-height: 320px;
    overflow-y: auto;
    padding: 0.25rem;
  }
  .link-picker-list .placeholder {
    padding: 1rem;
    color: var(--fg-faint);
    font-size: 0.78rem;
    text-align: center;
  }
  .link-picker-item {
    display: grid;
    grid-template-columns: auto 1fr auto;
    align-items: center;
    gap: 0.6rem;
    width: 100%;
    background: transparent;
    border: none;
    padding: 0.5rem 0.7rem;
    color: var(--fg);
    cursor: pointer;
    text-align: left;
    font: inherit;
    font-size: 0.8rem;
    border-radius: 2px;
  }
  .link-picker-item.active { background: var(--hover-bg); }
  .link-picker-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--fg-faint);
  }
  .link-picker-dot[data-color="amber"]  { background: #c08a3e; }
  .link-picker-dot[data-color="cobalt"] { background: #4a78c0; }
  .link-picker-dot[data-color="rose"]   { background: #c04a78; }
  .link-picker-dot[data-color="sage"]   { background: #6a9874; }
  .link-picker-dot[data-color="violet"] { background: #8a4ac0; }
  .link-picker-dot[data-color="slate"]  { background: #6a7480; }
  .link-picker-title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .link-picker-day {
    color: var(--fg-faint);
    font-family: "IBM Plex Mono", ui-monospace, monospace;
    font-size: 0.7rem;
  }
  .link-picker-hint {
    padding: 0.45rem 0.85rem;
    border-top: 1px solid var(--border);
    color: var(--fg-faint);
    font-size: 0.65rem;
    font-family: "IBM Plex Mono", ui-monospace, monospace;
    text-align: right;
  }

  /* Placeholder shown when editor is empty */
  :global(.editor .tiptap p.is-editor-empty:first-child::before) {
    content: attr(data-placeholder);
    color: var(--fg-faint);
    float: left;
    height: 0;
    pointer-events: none;
  }

  /* Consistent focus ring on interactive elements */
  :global(button:focus-visible),
  :global(input:focus-visible),
  :global(select:focus-visible),
  :global(textarea:focus-visible) {
    outline: 1px solid var(--fg-dim);
    outline-offset: 2px;
  }

  /* Toast stack (bottom-right) */
  .toast-stack {
    position: fixed;
    right: 1rem;
    bottom: 1rem;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    z-index: 1000;
    pointer-events: none;
  }
  .toast {
    pointer-events: auto;
    max-width: 360px;
    padding: 0.6rem 0.85rem;
    background: var(--panel);
    color: var(--fg);
    border: 1px solid var(--border-2);
    border-left: 3px solid #c08a3e;
    border-radius: 2px;
    font: inherit;
    font-size: 0.75rem;
    text-align: left;
    cursor: pointer;
    box-shadow: 0 4px 16px rgba(0,0,0,0.35);
    animation: toast-in 160ms ease-out;
  }
  .toast-error { border-left-color: #c2553a; }
  .toast:hover { background: var(--panel-2); }
  @keyframes toast-in {
    from { opacity: 0; transform: translateY(6px); }
    to { opacity: 1; transform: translateY(0); }
  }

  /* Scheduled-for-today banner above editor */
  .scheduled {
    margin-bottom: 1.5rem;
    padding: 0.5rem 0.75rem 0.6rem;
    border-left: 2px solid var(--fg-faint);
    background: var(--panel-2);
    border-radius: 0 2px 2px 0;
  }
  .scheduled-head {
    font-family: "IBM Plex Mono", ui-monospace, monospace;
    font-size: 0.6rem;
    color: var(--fg-faint);
    text-transform: uppercase;
    letter-spacing: 0.1em;
    margin-bottom: 0.4rem;
  }
  .scheduled-row {
    display: flex;
    align-items: baseline;
    gap: 0.5rem;
    padding: 0.15rem 0;
    font-size: 0.85rem;
  }
  .scheduled-text {
    flex: 1 1 auto;
    background: none;
    border: 0;
    padding: 0;
    color: var(--fg-dim);
    font: inherit;
    text-align: left;
    cursor: pointer;
  }
  .scheduled-text:hover {
    color: var(--fg);
    text-decoration: underline;
  }
  .scheduled-from {
    font-family: "IBM Plex Mono", ui-monospace, monospace;
    font-size: 0.6rem;
    color: var(--fg-faint);
    text-transform: uppercase;
    letter-spacing: 0.08em;
  }
</style>