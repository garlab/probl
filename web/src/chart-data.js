// Interpret versioned report summaries without parsing display labels. This is
// deliberately independent of the DOM, so chart eligibility and gaps are tested.
export const MAX_POINTS = 1000;
export const MAX_CHARTS = 20;
export const currencies = ['', 'USD', 'EUR', 'GBP'];
const finite = (n) => typeof n === 'number' && Number.isFinite(n);

export function chartSettings(value) {
  const reports = {};
  if (value?.version === 1 && value.reports && typeof value.reports === 'object') {
    for (const [id, choice] of Object.entries(value.reports).slice(0, 256)) {
      if (!/^\d{1,5}$/.test(id) || !choice || typeof choice !== 'object') continue;
      reports[id] = {
        baseline: Number.isSafeInteger(choice.baseline) && choice.baseline >= 0 ? choice.baseline : null,
        currency: currencies.includes(choice.currency) ? choice.currency : '',
        ...(choice.collection === 'moments' ? { collection: 'moments' } : {}),
        ...(typeof choice.summary === 'string' && choice.summary.length <= 4096 && choice.summary ? { summary: choice.summary } : {}),
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

function levelLabel(q) {
  // Shift the shortest decimal string instead of multiplying/rounding: nearby
  // requested levels must retain distinct labels, including tiny tail levels.
  const [mantissa, exponent = '0'] = String(q).split('e');
  const [whole, fraction = ''] = mantissa.split('.');
  const digits = whole + fraction;
  const point = whole.length + Number(exponent) + 2;
  const shifted = point <= 0 ? `0.${'0'.repeat(-point)}${digits}`
    : point >= digits.length ? digits + '0'.repeat(point - digits.length)
      : `${digits.slice(0, point)}.${digits.slice(point)}`;
  return `${shifted.replace(/^0+(?=\d)/, '').replace(/(\.\d*?)0+$/, '$1').replace(/\.$/, '')}%`;
}

export function columnEstimate(group, column) {
  if (column.kind === 'probability') return group.probability;
  if (column.kind === 'mean' || column.kind === 'sd') return group.summary?.[column.kind];
  return group.summary?.quantiles.find((q) => q.level === column.level
    && (q.convention ?? (q.level === 0.5 ? 'midpoint' : 'inverse_cdf')) === column.convention)?.estimate;
}

function columnValue(group, column) {
  const q = group.summary?.quantiles.find((q) => q.level === column.level
    && (q.convention ?? (q.level === 0.5 ? 'midpoint' : 'inverse_cdf')) === column.convention);
  // Exact integers which JS cannot represent must never enter a numeric axis.
  if (q?.value?.kind === 'int' && keyCoordinate(q.value) === null) return null;
  const value = columnEstimate(group, column)?.point;
  return finite(value) ? value : null;
}

function columnsFor(report) {
  if (report.groups[0].kind === 'probability') return [{ kind: 'probability', label: 'Probability' }];
  const tokens = report.summary_selection ?? ['q:0.05', 'q:0.25', 'median', 'q:0.75', 'q:0.95'];
  return tokens.map((token) => {
    if (token === 'mean' || token === 'sd') return { kind: token, label: token === 'mean' ? 'Mean' : 'SD' };
    const median = token === 'median';
    const level = median ? 0.5 : Number(token.slice(2));
    return { kind: 'quantile', level, convention: median ? 'midpoint' : 'inverse_cdf', label: median ? 'Median' : levelLabel(level) };
  });
}

/** Null means use the existing text view. Unknown or unsafe values stay gaps. */
export function seriesFor(report) {
  const groups = report.groups;
  if (!groups?.length || groups.length > MAX_POINTS) return null;
  const first = groups[0];
  if (!['number', 'probability'].includes(first.kind)) return null;
  const keyKind = first.key?.kind;
  const percent = first.kind === 'probability' || first.summary?.percent === true;
  const columns = columnsFor(report);
  // Prefer a midpoint median, then a mean, then the requested quantile closest
  // to half the mass. An SD-only request is explicitly labelled as SD.
  let primary = columns.findIndex((c) => c.convention === 'midpoint');
  if (primary < 0) primary = columns.findIndex((c) => c.kind === 'mean');
  if (primary < 0) {
    primary = 0;
    columns.forEach((c, i) => { if (c.kind === 'quantile' && Math.abs(c.level - 0.5) < Math.abs((columns[primary].level ?? Infinity) - 0.5)) primary = i; });
  }
  const quantiles = columns.map((c, i) => ({ ...c, index: i })).filter((c) => c.kind === 'quantile' && c.level !== 0.5).sort((a, b) => a.level - b.level);
  const bands = [];
  for (let lo = 0, hi = quantiles.length - 1; lo < hi; lo++, hi--) {
    const a = quantiles[lo], b = quantiles[hi];
    if (a.level < 0.5 && b.level > 0.5) bands.push({ lo: a.index, hi: b.index, label: `${levelLabel(a.level).slice(0, -1)}–${levelLabel(b.level)}` });
  }
  const points = [];
  for (const g of groups) {
    if (g.kind !== first.kind || g.key?.kind !== keyKind
      || (g.kind === 'number' && (g.summary?.percent === true) !== percent)) return null;
    const x = keyCoordinate(g.key);
    if (x === null) return null;
    const values = columns.map((c) => columnValue(g, c));
    points.push({ x, key: g.key, group: g, values, y: values[primary] });
  }
  points.sort((a, b) => a.x - b.x);
  // Even distinct typed keys must not collapse onto the same coordinate.
  if (points.some((p, i) => i > 0 && p.x <= points[i - 1].x)) return null;
  if (!points.some((p) => finite(p.y))) return null;
  return {
    report, points, keyKind, percent, columns, primary, bands, kind: first.kind,
    deterministic: first.kind === 'number' && columns[primary].kind !== 'sd' && groups.every((g) => g.summary?.deterministic === true),
  };
}

export function compatibleBaselines(series, all) {
  if (series.kind !== 'number' || series.columns[series.primary].kind === 'sd') return [];
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
