import type { CalCell, DayInfo } from "$lib/app-model";

export function dateKey(d: Date): string {
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

export function todayIso(): string {
  return dateKey(new Date());
}

export function tomorrowIso(): string {
  const d = new Date();
  d.setDate(d.getDate() + 1);
  return dateKey(d);
}

export function dueLabel(d: string): string {
  if (d === todayIso()) return "Today";
  if (d === tomorrowIso()) return "Tomorrow";
  const [y, m, dd] = d.split("-").map(Number);
  return new Date(y, m - 1, dd).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
  });
}

export function headlineLabel(date: string): string {
  if (!date) return "";
  const [y, m, d] = date.split("-").map(Number);
  return new Date(y, m - 1, d).toLocaleDateString(undefined, {
    weekday: "long",
    month: "long",
    day: "numeric",
  });
}

export function dayLabel(d: string, currentDate: string): string {
  if (d === currentDate) return "Today";
  const [y, m, dd] = d.split("-").map(Number);
  return new Date(y, m - 1, dd).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
  });
}

export function buildCalendarGrid(calendarMonth: string, days: DayInfo[]): CalCell[] {
  if (!calendarMonth) return [];
  const [y, m] = calendarMonth.split("-").map(Number);
  const first = new Date(y, m - 1, 1);
  const startWeekday = first.getDay();
  const gridStart = new Date(y, m - 1, 1 - startWeekday);
  const dayMap = new Map(days.map((d) => [d.date, d]));
  const cells: CalCell[] = [];

  for (let i = 0; i < 42; i++) {
    const dt = new Date(gridStart.getFullYear(), gridStart.getMonth(), gridStart.getDate() + i);
    const ds = dateKey(dt);
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
}

export function calendarLabel(calendarMonth: string): string {
  if (!calendarMonth) return "";
  const [y, m] = calendarMonth.split("-").map(Number);
  return new Date(y, m - 1, 1).toLocaleDateString(undefined, {
    month: "long",
    year: "numeric",
  });
}

export function escapeHtml(s: string): string {
  return s
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;");
}
