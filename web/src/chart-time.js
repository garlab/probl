// Calendar positions and unit-bearing magnitudes are exact until the final
// conversion to relative SVG coordinates. Never use epoch milliseconds for
// datetime identity: that would erase submillisecond differences.
export const timeKinds = ['date', 'datetime', 'duration', 'squared_duration'];
export const isTime = (kind) => timeKinds.includes(kind);
const units = [['w', 604800000000000n], ['d', 86400000000000n], ['h', 3600000000000n],
  ['m', 60000000000n], ['s', 1000000000n], ['ms', 1000000n], ['us', 1000n], ['ns', 1n]];
const decimal = (n, scale = 0) => {
  while (scale > 0 && n % 10n === 0n) { n /= 10n; scale--; }
  return { n, scale };
};
const atScale = (a, scale) => a.n * 10n ** BigInt(scale - a.scale);
const difference = (a, b) => {
  const scale = Math.max(a.scale, b.scale);
  return Number(`${atScale(a, scale) - atScale(b, scale)}e-${scale}`);
};

export function timeValue(value) {
  if (!isTime(value?.kind) || typeof value.value !== 'string' || value.value.length > 21000) return null;
  const text = value.value;
  if (value.kind === 'date' || value.kind === 'datetime') {
    const match = /^(\d{4}-\d{2}-\d{2})(?:T(\d{2}:\d{2}:\d{2})(?:\.(\d{1,9}))?)?$/.exec(text);
    if (!match || (value.kind === 'datetime') !== !!match[2] || match[1].startsWith('0000')) return null;
    const base = `${match[1]}T${match[2] ?? '00:00:00'}`;
    const ms = Date.parse(`${base}Z`);
    if (!Number.isFinite(ms) || new Date(ms).toISOString().slice(0, 19) !== base) return null;
    return decimal(value.kind === 'date' ? BigInt(ms / 86400000)
      : BigInt(ms) * 1000000n + BigInt((match[3] ?? '').padEnd(9, '0')));
  }
  // Canonical duration output is one decimal coefficient and unit. These are
  // typed JSON values, never display labels or source expressions.
  const match = /^(-?\d+)(?:\.(\d+))?(ns|us|ms|s|m|h|d|w)(\^2)?$/.exec(text);
  if (!match || (value.kind === 'squared_duration') !== !!match[4]) return null;
  const factor = units.find(([u]) => u === match[3])[1] ** (match[4] ? 2n : 1n);
  return decimal(BigInt(match[1] + (match[2] ?? '')) * factor, (match[2] ?? '').length);
}

function calendarText(kind, n) {
  if (kind === 'date') return new Date(Number(n) * 86400000).toISOString().slice(0, 10);
  let seconds = n / 1000000000n, fraction = n % 1000000000n;
  if (fraction < 0n) { seconds--; fraction += 1000000000n; }
  const base = new Date(Number(seconds) * 1000).toISOString().slice(0, 19);
  return base + (fraction ? `.${String(fraction).padStart(9, '0').replace(/0+$/, '')}` : '');
}

function durationTicks(magnitude, squared) {
  const power = squared ? 2 : 1;
  const [unit, base] = units.find(([, n]) => Math.abs(magnitude) >= Number(n) ** power) ?? units.at(-1);
  const factor = Number(base) ** power;
  const label = `${unit}${squared ? '²' : ''}`;
  const format = new Intl.NumberFormat('en', { maximumSignificantDigits: 6 });
  return (n) => [`${Math.abs(n / factor) >= 1e15 ? (n / factor).toExponential(3) : format.format(n / factor)}${label}`];
}

/** A shared axis origin, exact labels, and a collision check for plotted values. */
export function timeAxis(kind, values) {
  const parsed = values.filter((v) => v?.kind === kind).map((v) => [v, timeValue(v)]).filter(([, n]) => n);
  if (!parsed.length) return null;
  const scale = Math.max(...parsed.map(([, n]) => n.scale));
  const sorted = parsed.map(([v, n]) => [v, atScale(n, scale)]).sort((a, b) => a[1] < b[1] ? -1 : a[1] > b[1] ? 1 : 0);
  const min = decimal(sorted[0][1], scale), max = decimal(sorted.at(-1)[1], scale);
  const calendar = kind === 'date' || kind === 'datetime';
  let origin = calendar ? min : decimal(0n);
  const coordinate = (v) => {
    const n = v?.kind === kind ? timeValue(v) : null;
    const x = n ? difference(n, origin) : NaN;
    return Number.isFinite(x) ? x : null;
  };
  const canPlot = (values) => {
    const seen = new Map();
    for (const v of values) {
      const n = timeValue(v), x = coordinate(v);
      if (!n || x === null) return false;
      const identity = `${n.n}e-${n.scale}`;
      if (seen.has(x) && seen.get(x) !== identity) return false;
      seen.set(x, identity);
    }
    return true;
  };
  const valid = parsed.map(([v]) => v);
  if (!canPlot(valid)) {
    origin = min;
    if (!canPlot(valid)) return null;
  }
  const low = difference(min, origin), high = difference(max, origin);
  const shortClock = kind === 'datetime' && high - low < 60000000000;
  const relative = !calendar && origin.n !== 0n;
  const title = { date: 'Date', datetime: 'Datetime', duration: 'Duration', squared_duration: 'Squared duration' }[kind];
  const first = sorted[0][0].value, last = sorted.at(-1)[0].value;
  const context = shortClock ? `Datetime · offsets from ${first}`
    : relative ? `${title} · offsets from ${first}`
      : calendar ? `${title} · ${first.slice(0, 10)}${first.slice(0, 10) === last.slice(0, 10) ? '' : ` to ${last.slice(0, 10)}`}` : title;
  const ticks = (x, extent = [low, high], exactValue = null) => {
    if (!calendar || shortClock) return durationTicks(Math.max(...extent.map(Math.abs)), kind === 'squared_duration')(x);
    // Ticks are approximate, but endpoints and inspection use exact values.
    let n = origin.n + BigInt(Math.round(x));
    if (x === low) n = min.n;
    if (x === high) n = max.n;
    const exact = exactValue?.kind === kind ? timeValue(exactValue) : null;
    if (exact) n = exact.n;
    const text = calendarText(kind, n);
    return kind === 'datetime' ? [text.slice(0, 10), text.slice(11).replace(/\.\d+$/, '')] : [text];
  };
  return { kind, coordinate, canPlot, ticks, context, includeZero: !calendar && !relative };
}
