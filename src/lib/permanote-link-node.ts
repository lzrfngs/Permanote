// Inline atom node for `[[permanote:id|title]]` — clickable links between
// daily notes and a permanote.
//
// File representation (markdown): `[[permanote:8c1d|MGB GT carb notes]]`
// Editor representation: an inline atom rendering a styled chip.

import { Node, mergeAttributes } from "@tiptap/core";

declare module "@tiptap/core" {
  interface Commands<ReturnType> {
    permanoteLink: {
      insertPermanoteLink: (attrs: { id: string; title: string }) => ReturnType;
    };
  }
}

export const PermanoteLink = Node.create({
  name: "permanoteLink",
  group: "inline",
  inline: true,
  atom: true,
  selectable: true,

  addAttributes() {
    return {
      id: {
        default: "",
        parseHTML: (el) => el.getAttribute("data-id") || "",
        renderHTML: (attrs) => ({ "data-id": attrs.id }),
      },
      title: {
        default: "",
        parseHTML: (el) => el.getAttribute("data-title") || el.textContent || "",
        renderHTML: (attrs) => ({ "data-title": attrs.title }),
      },
    };
  },

  parseHTML() {
    return [{ tag: 'a[data-type="permanote-link"]' }];
  },

  renderHTML({ HTMLAttributes, node }) {
    const title = (node.attrs.title as string) || "permanote";
    return [
      "a",
      mergeAttributes(HTMLAttributes, {
        "data-type": "permanote-link",
        class: "permanote-link",
        href: `#permanote/${node.attrs.id}`,
      }),
      title,
    ];
  },

  addCommands() {
    return {
      insertPermanoteLink:
        (attrs) =>
        ({ commands }) =>
          commands.insertContent({
            type: this.name,
            attrs,
          }),
    };
  },

  addStorage() {
    return {
      markdown: {
        serialize(state: any, node: any) {
          const id = node.attrs.id || "";
          const title = (node.attrs.title || "").replace(/\]\]/g, "] ]");
          if (!id) return;
          state.write(`[[permanote:${id}|${title}]]`);
        },
        parse: {
          setup(markdownit: any) {
            // Inline rule: scan for `[[permanote:ID|TITLE]]` and emit a
            // self-closing token that the renderer turns into an anchor.
            markdownit.inline.ruler.before("link", "permanote_link", inlineRule);
            markdownit.renderer.rules.permanote_link = (tokens: any, idx: number) => {
              const t = tokens[idx];
              const id = escapeAttr(t.attrGet("id") || "");
              const title = escapeAttr(t.attrGet("title") || "");
              return `<a data-type="permanote-link" data-id="${id}" data-title="${title}">${title}</a>`;
            };
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

function inlineRule(state: any, silent: boolean): boolean {
  const src: string = state.src;
  const pos: number = state.pos;
  if (src.charCodeAt(pos) !== 0x5b /* [ */ || src.charCodeAt(pos + 1) !== 0x5b) {
    return false;
  }
  // Match `[[permanote:ID|TITLE]]`
  const slice = src.slice(pos);
  const m = /^\[\[permanote:([A-Za-z0-9_-]+)\|([^\]]*?)\]\]/.exec(slice);
  if (!m) return false;
  if (!silent) {
    const token = state.push("permanote_link", "", 0);
    token.attrSet("id", m[1]);
    token.attrSet("title", m[2]);
    token.markup = "[[permanote]]";
  }
  state.pos += m[0].length;
  return true;
}
