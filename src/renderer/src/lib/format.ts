/** m:ss. Tabular figures in the UI keep these from jittering as they count. */
export function fmtTime(seconds: number): string {
  const t = Math.max(0, Math.floor(seconds || 0))
  return `${Math.floor(t / 60)}:${String(t % 60).padStart(2, '0')}`
}
