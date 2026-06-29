export type Settings = {
  vault_root: string | null;
  permanote_mode: "color" | "label";
  theme: "light" | "dark" | "system";
  permanote_order: string[];
};

export type Todo = {
  day: string;
  line: number;
  id: string;
  text: string;
  done: boolean;
  due?: string | null;
};

export type Permanote = {
  id: string;
  day: string;
  line: number;
  color: string;
  title: string;
  snippet: string;
};

export type DayInfo = {
  date: string;
  has_open_todos: boolean;
};

export type SearchHit = {
  date: string;
  snippet: string;
};

export type Toast = {
  id: number;
  message: string;
  kind: "error" | "info";
};

export type PermanoteFile = {
  id: string;
  title: string;
  color: string;
  source_day: string;
  created: string;
  modified: string;
  content: string;
};

export type CalCell = {
  date: string;
  day: number;
  inMonth: boolean;
  hasContent: boolean;
  hasOpen: boolean;
};
