/** Human byte size. Mirrors the engine's `telemetry::human_bytes` so the feed and
 *  the table never disagree about the same number. */
export function bytes(n: number): string {
  if (!n || n <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let v = n;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return i >= 2 ? `${v.toFixed(2)} ${units[i]}` : `${Math.round(v)} ${units[i]}`;
}

/** Split the number from its unit so the UI can style them differently. */
export function bytesParts(n: number): [string, string] {
  const s = bytes(n);
  const i = s.lastIndexOf(" ");
  return [s.slice(0, i), s.slice(i + 1)];
}

export function thousands(n: number): string {
  return n.toLocaleString("en-US");
}

/** Elapsed microseconds as a stopwatch reading for the telemetry gutter. */
export function stamp(us: number): string {
  const s = us / 1e6;
  return s.toFixed(4).padStart(7, "0") + "s";
}

export function idle(days: number): string {
  if (days <= 0) return "today";
  if (days === 1) return "yesterday";
  if (days < 30) return `${days}d idle`;
  if (days < 365) return `${Math.floor(days / 30)}mo idle`;
  return `${(days / 365).toFixed(1)}y idle`;
}

/** Shorten a path for display, keeping the tail, which is the identifying part. */
export function shortPath(p: string, home: string): string {
  let s = p.startsWith(home) ? "~" + p.slice(home.length) : p;
  if (s.length <= 52) return s;
  const parts = s.split("/");
  if (parts.length <= 4) return s;
  return `${parts[0]}/${parts[1]}/…/${parts.slice(-2).join("/")}`;
}
