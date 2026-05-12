// Custom Tiptap node for "Permanote" — a labeled, colored block that wraps
// arbitrary content (paragraphs, lists, etc.) and round-trips to a markdown
// fence so the file stays human-readable.
//
// File representation:
//   %%permanote-start id=8c1d color=amber title="MGB GT carb rebuild notes"%%
//   ...content...
//   %%permanote-end id=8c1d%%

import { Node, mergeAttributes } from "@tiptap/core";

export interface PermanoteAttrs {
  id: string;
  color: string;
  title: string;
}

export const COLORS = ["amber", "cobalt", "rose", "sage", "violet", "slate"] as const;

function generateId(): string {
  // Short hex id, e.g. "8c1d"
  return Math.floor(Math.random() * 0xffff).toString(16).padStart(4, "0");
}

declare module "@tiptap/core" {
  interface Commands<ReturnType> {
    permanote: {
      wrapSelectionInPermanote: (attrs?: Partial<PermanoteAttrs>) => ReturnType;
      unwrapPermanote: () => ReturnType;
    };
  }
}

export const Permanote = Node.create({
  name: "permanote",
  group: "block",
  content: "block+",
  defining: true,
  isolating: true,

  addAttributes() {
    return {
      id: {
        default: null,
        parseHTML: (el) => el.getAttribute("data-id"),
        renderHTML: (attrs) => ({ "data-id": attrs.id }),
      },
      color: {
        default: "amber",
        parseHTML: (el) => el.getAttribute("data-color") || "amber",
        renderHTML: (attrs) => ({ "data-color": attrs.color }),
      },
      title: {
        default: "",
        parseHTML: (el) => el.getAttribute("data-title") || "",
        renderHTML: (attrs) => ({ "data-title": attrs.title }),
      },
    };
  },

  parseHTML() {
    return [{ tag: "div[data-type=\"permanote\"]" }];
  },

  renderHTML({ HTMLAttributes, node }) {
    const title = (node.attrs.title as string) || "Permanote";
    return [
      "div",
      mergeAttributes(HTMLAttributes, {
        "data-type": "permanote",
        class: "permanote",
      }),
      [
        "div",
        { class: "permanote-head", contenteditable: "false" },
        ["span", { class: "permanote-title" }, title],
        ["span", { class: "permanote-id" }, node.attrs.id || ""],
      ],
      ["div", { class: "permanote-body" }, 0],
    ];
  },

  addNodeView() {
    return ({ node, getPos, editor }) => {
      const dom = document.createElement("div");
      dom.setAttribute("data-type", "permanote");
      dom.className = "permanote";
      dom.setAttribute("data-id", node.attrs.id || "");
      dom.setAttribute("data-color", node.attrs.color || "amber");
      dom.setAttribute("data-title", node.attrs.title || "");

      const head = document.createElement("div");
      head.className = "permanote-head";
      head.setAttribute("contenteditable", "false");

      const title = document.createElement("span");
      title.className = "permanote-title";
      title.setAttribute("contenteditable", "true");
      title.setAttribute("spellcheck", "false");
      title.textContent = node.attrs.title || "";
      title.dataset.placeholder = "Untitled";

      // Commit title on blur or Enter.
      const commitTitle = () => {
        const next = (title.textContent || "").trim();
        const pos = typeof getPos === "function" ? getPos() : null;
        if (pos == null) return;
        const current = (editor.state.doc.nodeAt(pos)?.attrs.title) ?? "";
        if (next === current) return;
        const tr = editor.state.tr.setNodeMarkup(pos, undefined, {
          ...editor.state.doc.nodeAt(pos)!.attrs,
          title: next,
        });
        editor.view.dispatch(tr);
      };
      title.addEventListener("blur", commitTitle);
      title.addEventListener("keydown", (e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          (e.target as HTMLElement).blur();
        }
      });

      const swatchWrap = document.createElement("div");
      swatchWrap.className = "permanote-swatch-wrap";
      swatchWrap.setAttribute("contenteditable", "false");

      const swatch = document.createElement("button");
      swatch.type = "button";
      swatch.className = "permanote-swatch";
      swatch.setAttribute("aria-label", "Change color");
      swatch.title = "Change color";

      const popover = document.createElement("div");
      popover.className = "permanote-color-popover";
      popover.setAttribute("role", "listbox");
      popover.hidden = true;

      const closePopover = () => {
        popover.hidden = true;
        document.removeEventListener("mousedown", onOutside, true);
      };
      const onOutside = (e: MouseEvent) => {
        if (!swatchWrap.contains(e.target as Node)) closePopover();
      };
      swatch.addEventListener("mousedown", (e) => {
        e.preventDefault();
        if (popover.hidden) {
          popover.hidden = false;
          document.addEventListener("mousedown", onOutside, true);
        } else {
          closePopover();
        }
      });

      for (const c of COLORS) {
        const d = document.createElement("button");
        d.type = "button";
        d.className = "permanote-dot";
        d.dataset.color = c;
        d.title = c;
        d.setAttribute("aria-label", `Set color ${c}`);
        d.addEventListener("mousedown", (e) => {
          e.preventDefault();
          const pos = typeof getPos === "function" ? getPos() : null;
          if (pos == null) { closePopover(); return; }
          const current = editor.state.doc.nodeAt(pos);
          if (!current) { closePopover(); return; }
          if (current.attrs.color !== c) {
            const tr = editor.state.tr.setNodeMarkup(pos, undefined, {
              ...current.attrs,
              color: c,
            });
            editor.view.dispatch(tr);
          }
          closePopover();
        });
        popover.appendChild(d);
      }

      swatchWrap.appendChild(swatch);
      swatchWrap.appendChild(popover);

      const idEl = document.createElement("span");
      idEl.className = "permanote-id";
      idEl.textContent = node.attrs.id || "";

      head.appendChild(title);
      head.appendChild(swatchWrap);
      head.appendChild(idEl);

      const body = document.createElement("div");
      body.className = "permanote-body";

      dom.appendChild(head);
      dom.appendChild(body);

      return {
        dom,
        contentDOM: body,
        update(updated: any) {
          if (updated.type.name !== "permanote") return false;
          dom.setAttribute("data-color", updated.attrs.color || "amber");
          dom.setAttribute("data-title", updated.attrs.title || "");
          dom.setAttribute("data-id", updated.attrs.id || "");
          if (document.activeElement !== title) {
            const t = updated.attrs.title || "";
            if (title.textContent !== t) title.textContent = t;
          }
          idEl.textContent = updated.attrs.id || "";
          return true;
        },
        ignoreMutation(mutation: any) {
          // Don't let mutations inside the head bubble up as ProseMirror edits.
          return head.contains(mutation.target);
        },
        stopEvent(event: Event) {
          // Keep clicks/keys inside the head from being interpreted as editor input.
          return head.contains(event.target as Node);
        },
      };
    };
  },

  addCommands() {
    return {
      wrapSelectionInPermanote:
        (attrs = {}) =>
        ({ commands }) => {
          const finalAttrs: PermanoteAttrs = {
            id: attrs.id ?? generateId(),
            color: attrs.color ?? "amber",
            title: attrs.title ?? "Untitled",
          };
          return commands.wrapIn(this.type, finalAttrs);
        },
      unwrapPermanote:
        () =>
        ({ commands }) => commands.lift(this.type),
    };
  },

  addKeyboardShortcuts() {
    return {
      "Mod-Shift-P": () => this.editor.commands.wrapSelectionInPermanote(),
    };
  },

  addStorage() {
    return {
      markdown: {
        serialize(state: any, node: any) {
          const id = node.attrs.id || generateId();
          const color = node.attrs.color || "amber";
          const title = (node.attrs.title || "").replace(/"/g, '\\"');
          state.write(`%%permanote-start id=${id} color=${color} title="${title}"%%\n`);
          state.renderContent(node);
          state.write(`%%permanote-end id=${id}%%`);
          state.closeBlock(node);
        },
        parse: {
          setup(markdownit: any) {
            markdownit.block.ruler.before(
              "paragraph",
              "permanote",
              permanoteBlockRule,
              { alt: ["paragraph", "reference", "blockquote", "list"] },
            );
            markdownit.renderer.rules.permanote_open = (tokens: any, idx: number) => {
              const t = tokens[idx];
              const id = escapeAttr(t.attrGet("id") || "");
              const color = escapeAttr(t.attrGet("color") || "amber");
              const title = escapeAttr(t.attrGet("title") || "");
              return `<div data-type="permanote" data-id="${id}" data-color="${color}" data-title="${title}">\n`;
            };
            markdownit.renderer.rules.permanote_close = () => `</div>\n`;
          },
        },
      },
    };
  },
});

function escapeAttr(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/"/g, "&quot;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

// Parse %%permanote-start id=... color=... title="..."%%
// up through matching %%permanote-end id=...%%, recursively allowing the
// body to be parsed by markdown-it as nested block content.
function permanoteBlockRule(state: any, startLine: number, endLine: number, silent: boolean): boolean {
  const pos = state.bMarks[startLine] + state.tShift[startLine];
  const max = state.eMarks[startLine];
  const line = state.src.slice(pos, max);

  const startMatch = /^%%permanote-start\s+(.*?)%%\s*$/.exec(line);
  if (!startMatch) return false;
  if (silent) return true;

  const attrs = parseAttrs(startMatch[1]);

  // Find matching end line.
  let nextLine = startLine + 1;
  let endMatchLine = -1;
  while (nextLine < endLine) {
    const lpos = state.bMarks[nextLine] + state.tShift[nextLine];
    const lmax = state.eMarks[nextLine];
    const lline = state.src.slice(lpos, lmax);
    if (/^%%permanote-end(\s|%%)/.test(lline)) {
      endMatchLine = nextLine;
      break;
    }
    nextLine++;
  }
  if (endMatchLine === -1) return false;

  const openTok = state.push("permanote_open", "div", 1);
  openTok.block = true;
  openTok.markup = "%%";
  openTok.map = [startLine, endMatchLine + 1];
  openTok.attrSet("id", attrs.id || "");
  openTok.attrSet("color", attrs.color || "amber");
  openTok.attrSet("title", attrs.title || "");

  // Nested parse of the body (between start and end fences).
  state.md.block.tokenize(state, startLine + 1, endMatchLine);

  const closeTok = state.push("permanote_close", "div", -1);
  closeTok.block = true;
  closeTok.markup = "%%";

  state.line = endMatchLine + 1;
  return true;
}

// Parse `key=value` and `key="quoted value"` pairs.
function parseAttrs(input: string): Record<string, string> {
  const out: Record<string, string> = {};
  const re = /(\w+)=(?:"((?:[^"\\]|\\.)*)"|(\S+))/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(input)) !== null) {
    out[m[1]] = (m[2] !== undefined ? m[2].replace(/\\"/g, '"') : m[3]) ?? "";
  }
  return out;
}
