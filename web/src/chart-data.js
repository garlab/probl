// Interpret versioned report summaries without parsing display labels. This is
// deliberately independent of the DOM, so chart eligibility and gaps are tested.
export const MAX_POINTS = 1000;
export const MAX_CHARTS = 20;
export const currencies = ['', 'USD', 'EUR', 'GBP'];
const finite = (n) => typeof n === 'number' && Number.isFinite(n);

export function chartSettings(value) {
  const reports = {};
  if (value?.version === 1 && value.reports && typeof value.reports === 'object') {
    for (const [id, choice] of Object.entries(value.reports).slice(0, MAX_CHARTS)) {
      if (!/^\d{1,5}$/.test(id) || !choice || typeof choice !== 'object') continue;
      reports[id] = {
        baseline: Number.isSafeInteger(choice.baseline) && choice.baseline >= 0 ? choice.baseline : null,
        currency: currencies.includes(choice.currency) ? choice.currency : '',
      };
    }
  }
  return { version: 1, reports };
}

export function readChartSettings(text) {
  try { return chartSettings(JSON.parse(text)); } catch { return chartSettings(null); }
}

function keyCoordinate(key) {
  if (!key) return null;
  if (key.kind === 'int') {
    const n = Number(key.value);
    return /^-?\d+$/.test(key.value) && Number.isSafeInteger(n) ? n : null;
  }
  if (key.kind === 'float' || key.kind === 'prob') return finite(key.value) ? key.value : null;
  if (key.kind === 'date' && /^\d{4}-\d{2}-\d{2}$/.test(key.value)) {
    const n = Date.parse(`${key.value}T00:00:00Z`);
    return finite(n) ? n / 86400000 : null;
  }
  return null;
}

export const keyId = (key) => JSON.stringify([key.kind, key.value]);

function quantile(group, level) {
  const q = group.summary?.quantiles.find((q) => q.level === level);
  // Exact integers which JS cannot represent must never enter a numeric axis.
  if (q?.value?.kind === 'int' && keyCoordinate(q.value) === null) return null;
  const value = q?.estimate?.point;
  return finite(value) ? value : null;
}

/** Null means use the existing text view. Unknown or unsafe values stay gaps. */
export function seriesFor(report) {
  const groups = report.groups;
  if (!groups?.length || groups.length > MAX_POINTS) return null;
  const first = groups[0];
  if (!['number', 'probability'].includes(first.kind)) return null;
  const keyKind = first.key?.kind;
  const percent = first.kind === 'probability' || first.summary?.percent === true;
  const points = [];
  for (const g of groups) {
    if (g.kind !== first.kind || g.key?.kind !== keyKind
      || (g.kind === 'number' && (g.summary?.percent === true) !== percent)) return null;
    const x = keyCoordinate(g.key);
    if (x === null) return null;
    const values = first.kind === 'probability' ? [g.probability?.point] : [0.05, 0.25, 0.5, 0.75, 0.95].map((q) => quantile(g, q));
    points.push({ x, key: g.key, group: g, values, y: first.kind === 'probability' ? values[0] : values[2] });
  }
  points.sort((a, b) => a.x - b.x);
  // Even distinct typed keys must not collapse onto the same coordinate.
  if (points.some((p, i) => i > 0 && p.x <= points[i - 1].x)) return null;
  if (!points.some((p) => finite(p.y))) return null;
  return {
    report, points, keyKind, percent, kind: first.kind,
    deterministic: first.kind === 'number' && groups.every((g) => g.summary?.deterministic === true),
  };
}

export function compatibleBaselines(series, all) {
  if (series.kind !== 'number') return [];
  const keys = new Set(series.points.filter((p) => finite(p.y)).map((p) => keyId(p.key)));
  return all.filter((candidate) => candidate.report.id !== series.report.id
    && candidate.deterministic && candidate.keyKind === series.keyKind && candidate.percent === series.percent
    && candidate.points.some((p) => finite(p.y) && keys.has(keyId(p.key))));
}

/** Align only exact typed keys. An absent baseline point breaks the line. */
export function alignedBaseline(series, baseline) {
  const byKey = new Map(baseline?.points.map((p) => [keyId(p.key), p]) ?? []);
  return series.points.map((p) => ({ ...p, y: finite(p.y) ? byKey.get(keyId(p.key))?.y ?? null : null }));
}

/** Split paths at unavailable values; no interpolation across these gaps. */
export function segments(points, values = (p) => [p.y]) {
  const out = [];
  let current = [];
  for (const point of points) {
    if (values(point).every(finite)) current.push(point);
    else if (current.length) { out.push(current); current = []; }
  }
  if (current.length) out.push(current);
  return out;
}

export function chartSeries(document) {
  if (document?.schema_version !== 1) return [];
  return document.reports.map(seriesFor).filter(Boolean).slice(0, MAX_CHARTS);
}
