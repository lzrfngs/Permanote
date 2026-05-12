<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { Editor } from "@tiptap/core";
  import { Extension, InputRule } from "@tiptap/core";
  import StarterKit from "@tiptap/starter-kit";
  import TaskList from "@tiptap/extension-task-list";
  import TaskItem from "@tiptap/extension-task-item";
  import { Markdown } from "tiptap-markdown";
  import { Permanote } from "$lib/permanote-node";
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
  let frontmatter = "";
  let dirty = $state(false);
  let externalConflict = $state<{ date: string; content: string } | null>(null);
  let focusMode = $state(false);
  let unlistenExternal: UnlistenFn | null = null;

  type Todo = { day: string; line: number; id: string; text: string; done: boolean };
  let todos = $state<Todo[]>([]);
  let todoFilter = $state<"open" | "done" | "all">("open");

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
  let permaSort = $state<"recent" | "title">("recent");

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
    } else {
      // recent: by day desc
      list = [...list].sort((a, b) => (a.day < b.day ? 1 : a.day > b.day ? -1 : 0));
    }
    return list;
  });

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
    }
  }

  type SearchHit = { date: string; snippet: string };  let searchQuery = $state("");
  let searchHits = $state<SearchHit[]>([]);
  let searchTimer: ReturnType<typeof setTimeout> | null = null;

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
    }
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
      const body = editor.storage.markdown.getMarkdown();
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
    date = await invoke<string>("today");
    vaultPath = await invoke<string>("vault_path");
    calendarMonth = date.slice(0, 7);
    window.addEventListener("keydown", onKeydown);

    editor = new Editor({
      element: editorEl,
      extensions: [
        StarterKit,
        TaskList,
        TaskItemWithId.configure({ nested: true }),
        TaskListShortcut,
        Permanote,
        SlashCommand,
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
      onUpdate: scheduleSave,
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
    unlistenExternal?.();
    editor?.destroy();
  });

  function resolveKeepMine() {
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

  // --- Window chrome -----------------------------------------------------
  const appWindow = getCurrentWindow();
  function winMinimize() { appWindow.minimize(); }
  function winToggleMax() { appWindow.toggleMaximize(); }
  function winClose() { appWindow.close(); }

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
              {#if cell.hasContent}<span class="cal-dot dot-content"></span>{/if}
              {#if cell.hasOpen}<span class="cal-dot dot-open"></span>{/if}
            </span>
          </button>
        {/each}
      </div>
      <button class="cal-today" onclick={jumpToToday}>Today</button>
    </div>
    <div class="cal-foot">
      <button
        class="vault"
        title="Open vault folder in Explorer"
        onclick={() => invoke('open_vault_folder').catch(console.error)}
      >
        <svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.3" aria-hidden="true">
          <path d="M1.5 4.5a1 1 0 0 1 1-1h3.2l1.3 1.5h6.5a1 1 0 0 1 1 1v6.5a1 1 0 0 1-1 1h-11a1 1 0 0 1-1-1V4.5Z"/>
        </svg>
        <span class="vault-label">Open vault folder</span>
      </button>
      <span class="vault-path" title={vaultPath}>{vaultPath}</span>
    </div>
  </aside>

  <main class="canvas">
    <button
      class="focus-toggle"
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
    <header>
      <h1>{headline}</h1>
      <div class="meta">
        <span class="date">{date}</span>
        <span class="status status-{status}">{status}</span>
      </div>
      <div class="search">
        <input
          type="text"
          placeholder="Search…"
          bind:value={searchQuery}
          oninput={onSearchInput}
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
    </header>

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
                </div>
              {/each}
            </div>
          {/each}
        {/if}
      </div>
    </section>
    <section class="panel-section">
      <div class="panel-head">
        <span>Permanotes</span>
        <button
          class="perma-sort"
          onclick={() => (permaSort = permaSort === "recent" ? "title" : "recent")}
          title="Toggle sort"
        >{permaSort === "recent" ? "↓ date" : "A–Z"}</button>
      </div>
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
      <div class="panel-body">
        {#if filteredPermanotes.length === 0}
          <div class="placeholder">
            {permanotes.length === 0 ? "No permanotes yet." : "No matches."}
          </div>
        {:else}
          {#each filteredPermanotes as p (p.day + ":" + p.id)}
            <button
              class="perma"
              data-color={p.color}
              onclick={() => jumpToDay(p.day)}
              title="Jump to {p.day}"
            >
              <div class="perma-head">
                <span class="perma-title">{p.title || "Untitled"}</span>
                <span class="perma-day">{dayLabel(p.day)}</span>
              </div>
              {#if p.snippet}
                <div class="perma-snippet">{p.snippet}</div>
              {/if}
            </button>
          {/each}
        {/if}
      </div>
    </section>
  </aside>
</div>
</div>

<style>
  :global(:root) {
    color-scheme: dark;
    --bg: #0e0e0e;
    --fg: #e8e8e8;
    --fg-dim: #888;
    --fg-faint: #555;
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
    background: #0a0a0a;
    border-color: #1a1a1a;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  .panel-calendar {
    border-right: 1px solid #1a1a1a;
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
    color: var(--fg);
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
    background: #181818;
  }
  .cal-cell.out .cal-num {
    color: var(--fg-faint);
  }
  .cal-cell.selected {
    background: #1f1f1f;
    border-color: #333;
    color: var(--fg);
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
    border: 1px solid #222;
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
    border-color: #333;
    color: var(--fg);
  }

  .panel-right {
    border-left: 1px solid #1a1a1a;
  }

  .panel-section {
    display: flex;
    flex-direction: column;
    flex: 1 1 0;
    min-height: 0;
    overflow: hidden;
  }

  .panel-section + .panel-section {
    border-top: 1px solid #1a1a1a;
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
    border: 1px solid #1a1a1a;
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
    background: #1a1a1a;
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
  }
  .perma-sort:hover {
    color: var(--fg);
  }
  .perma-filter {
    padding: 0 0.85rem 0.5rem;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .perma-search {
    background: #0a0a0a;
    border: 1px solid #1a1a1a;
    color: var(--fg);
    font: inherit;
    font-size: 0.7rem;
    padding: 4px 6px;
    border-radius: 2px;
    outline: none;
  }
  .perma-search:focus {
    border-color: #333;
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
    background: var(--chip-color, #333);
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
    background: #131313;
    border: 1px solid #2a2a2a;
    border-left: 3px solid var(--accent, #c08a3e);
    padding: 0.5rem 0.75rem;
    margin-bottom: 0.5rem;
    cursor: pointer;
    color: inherit;
    font: inherit;
    border-radius: 2px;
  }

  .perma[data-color="amber"]  { --accent: #c08a3e; }
  .perma[data-color="cobalt"] { --accent: #4a78c0; }
  .perma[data-color="rose"]   { --accent: #c04a78; }
  .perma[data-color="sage"]   { --accent: #6a9874; }
  .perma[data-color="violet"] { --accent: #8a4ac0; }
  .perma[data-color="slate"]  { --accent: #6a7480; }

  .perma:hover {
    border-color: #3a3a3a;
    border-left-color: var(--accent);
  }

  .perma-head {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: 0.5rem;
    margin-bottom: 0.25rem;
  }

  .perma-title {
    font-size: 0.85rem;
    font-weight: 500;
    color: var(--accent);
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
    -webkit-box-orient: vertical;
  }

  .panel-body {
    flex: 1 1 auto;
    overflow-y: auto;
    padding: 0.5rem 1.25rem 1.25rem;
  }

  .placeholder {
    color: var(--fg-faint);
    font-size: 0.8rem;
    font-style: italic;
  }

  .canvas {
    max-width: 760px;
    width: 100%;
    margin: 0 auto;
    padding: 4rem 2rem 2rem;
    height: 100vh;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    box-sizing: border-box;
    position: relative;
  }
  .focus-toggle {
    position: absolute;
    top: 1rem;
    right: 1rem;
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
    z-index: 5;
  }
  .focus-toggle:hover {
    color: var(--fg);
    border-color: #2a2a2a;
    background: #161616;
  }
  .focus-toggle[aria-pressed="true"] {
    color: var(--fg);
    border-color: #2a2a2a;
  }

  header {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    margin-bottom: 2rem;
    border-bottom: 1px solid #222;
    padding-bottom: 1rem;
  }

  h1 {
    font-size: 1.5rem;
    font-weight: 600;
    margin: 0;
    letter-spacing: -0.01em;
  }

  .meta {
    display: flex;
    gap: 1rem;
    font-family: "IBM Plex Mono", ui-monospace, monospace;
    font-size: 0.7rem;
    color: var(--fg-dim);
    text-transform: uppercase;
    letter-spacing: 0.08em;
  }

  .status-saving { color: var(--fg-dim); }
  .status-saved { color: var(--fg-faint); }
  .status-error { color: #ff5a5a; }
  .status-loading { color: var(--fg-dim); }

  .search {
    position: relative;
    flex: 0 0 220px;
    margin-left: 1rem;
  }
  .search input {
    width: 100%;
    background: #141414;
    border: 1px solid #222;
    color: var(--fg);
    font: inherit;
    font-size: 0.8rem;
    padding: 0.35rem 0.6rem;
    border-radius: 2px;
    outline: none;
  }
  .search input:focus {
    border-color: #333;
  }
  .search-results {
    position: absolute;
    top: calc(100% + 4px);
    right: 0;
    width: 360px;
    max-height: 60vh;
    overflow-y: auto;
    background: #111;
    border: 1px solid #2a2a2a;
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
    border-bottom: 1px solid #1a1a1a;
    color: var(--fg);
    padding: 0.6rem 0.75rem;
    cursor: pointer;
    font: inherit;
  }
  .search-hit:hover {
    background: #181818;
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
    -webkit-box-orient: vertical;
  }
  :global(.hit-snippet mark) {
    background: #c08a3e;
    color: #0a0a0a;
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
    border-left: 2px solid #333;
    padding-left: 1rem;
    color: var(--fg-dim);
    margin: 0 0 0.75rem;
  }

  :global(.editor .permanote) {
    border: 1px solid #2a2a2a;
    border-left: 3px solid var(--accent, #c08a3e);
    border-radius: 2px;
    padding: 0.75rem 1rem;
    margin: 1rem 0;
    background: #131313;
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
    background: #111;
    border: 1px solid #2a2a2a;
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
    background: var(--dot-color, #444);
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
    color: var(--fg-faint);
    font-size: 0.6rem;
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
    border: 1px solid #2a2a2a;
    color: var(--fg);
    font: inherit;
    font-size: 0.7rem;
    padding: 3px 10px;
    cursor: pointer;
    border-radius: 2px;
  }
  .conflict-actions button:hover {
    background: #1d1d1d;
    border-color: #444;
  }

  .slash-menu {
    position: fixed;
    z-index: 1000;
    min-width: 220px;
    max-height: 280px;
    overflow-y: auto;
    background: #111;
    border: 1px solid #2a2a2a;
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
    background: #1d1d1d;
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
    background: #1a1a1a;
    padding: 0.1em 0.3em;
    border-radius: 2px;
  }

  .cal-foot {
    margin-top: auto;
    padding: 0.75rem 1rem 1rem;
    border-top: 1px solid #222;
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
  }
  .vault {
    display: inline-flex;
    align-items: center;
    gap: 0.5rem;
    background: transparent;
    border: 1px solid #2a2a2a;
    border-radius: 4px;
    padding: 0.45rem 0.6rem;
    color: var(--fg-dim);
    font-family: "IBM Plex Sans", ui-sans-serif, system-ui, sans-serif;
    font-size: 0.7rem;
    cursor: pointer;
    text-align: left;
    transition: border-color 120ms ease, color 120ms ease, background 120ms ease;
  }
  .vault svg {
    flex: 0 0 auto;
    color: var(--fg-faint);
  }
  .vault-label {
    flex: 1 1 auto;
  }
  .vault:hover {
    color: var(--fg);
    border-color: #444;
    background: #161616;
  }
  .vault:hover svg {
    color: var(--fg);
  }
  .vault-path {
    font-family: "IBM Plex Mono", ui-monospace, monospace;
    font-size: 0.6rem;
    color: var(--fg-faint);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
