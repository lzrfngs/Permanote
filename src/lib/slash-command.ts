import { Extension, type Editor, type Range } from "@tiptap/core";
import Suggestion, { type SuggestionOptions } from "@tiptap/suggestion";

export type SlashItem = {
  title: string;
  hint?: string;
  keywords?: string[];
  command: (ctx: { editor: Editor; range: Range }) => void;
};

export const slashItems: SlashItem[] = [
  {
    title: "Todo",
    hint: "Task with checkbox",
    keywords: ["task", "check", "todo"],
    command: ({ editor, range }) => {
      editor.chain().focus().deleteRange(range).toggleTaskList().run();
    },
  },
  {
    title: "Permanote",
    hint: "Pinned card",
    keywords: ["perma", "pin", "card"],
    command: ({ editor, range }) => {
      editor
        .chain()
        .focus()
        .deleteRange(range)
        // @ts-expect-error custom command
        .wrapSelectionInPermanote()
        .run();
    },
  },
  {
    title: "Heading 1",
    hint: "Large heading",
    keywords: ["h1", "title"],
    command: ({ editor, range }) => {
      editor.chain().focus().deleteRange(range).setNode("heading", { level: 1 }).run();
    },
  },
  {
    title: "Heading 2",
    keywords: ["h2"],
    command: ({ editor, range }) => {
      editor.chain().focus().deleteRange(range).setNode("heading", { level: 2 }).run();
    },
  },
  {
    title: "Heading 3",
    keywords: ["h3"],
    command: ({ editor, range }) => {
      editor.chain().focus().deleteRange(range).setNode("heading", { level: 3 }).run();
    },
  },
  {
    title: "Bullet list",
    keywords: ["ul", "list", "bullets"],
    command: ({ editor, range }) => {
      editor.chain().focus().deleteRange(range).toggleBulletList().run();
    },
  },
  {
    title: "Numbered list",
    keywords: ["ol", "ordered"],
    command: ({ editor, range }) => {
      editor.chain().focus().deleteRange(range).toggleOrderedList().run();
    },
  },
  {
    title: "Quote",
    keywords: ["blockquote"],
    command: ({ editor, range }) => {
      editor.chain().focus().deleteRange(range).toggleBlockquote().run();
    },
  },
  {
    title: "Divider",
    hint: "Horizontal rule",
    keywords: ["hr", "rule", "separator"],
    command: ({ editor, range }) => {
      editor.chain().focus().deleteRange(range).setHorizontalRule().run();
    },
  },
];

export type SlashRenderProps = {
  items: SlashItem[];
  command: (item: SlashItem) => void;
  clientRect: (() => DOMRect | null) | null;
  query: string;
};

export type SlashRenderHandlers = {
  onStart: (props: SlashRenderProps) => void;
  onUpdate: (props: SlashRenderProps) => void;
  onKeyDown: (props: { event: KeyboardEvent }) => boolean;
  onExit: () => void;
};

export function createSlashCommand(render: () => SlashRenderHandlers) {
  return Extension.create({
    name: "slashCommand",
    addOptions() {
      return {
        suggestion: {
          char: "/",
          startOfLine: false,
          allowSpaces: false,
          command: ({ editor, range, props }: any) => {
            (props as SlashItem).command({ editor, range });
          },
          items: ({ query }: { query: string }) => {
            const q = query.toLowerCase().trim();
            if (!q) return slashItems;
            return slashItems.filter((it) => {
              if (it.title.toLowerCase().includes(q)) return true;
              return (it.keywords || []).some((k) => k.includes(q));
            });
          },
          render,
        } satisfies Partial<SuggestionOptions<SlashItem>>,
      };
    },
    addProseMirrorPlugins() {
      return [
        Suggestion({
          editor: this.editor,
          ...this.options.suggestion,
        }),
      ];
    },
  });
}
