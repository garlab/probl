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

// Match Rust's fixed-point formatting, including ties to even, from the actual
// binary64 value. Rounding its shortest decimal string can choose the other
// side of a half-cent boundary (and Intl's default rounds exact ties away).
const floatBits = new DataView(new ArrayBuffer(8));
function decimalText(rounded, places, negative) {
  const digits = String(rounded).padStart(places + 1, '0');
  const whole = (places ? digits.slice(0, -places) : digits).replace(/\B(?=(\d{3})+(?!\d))/g, ',');
  return `${negative && rounded ? '-' : ''}${whole}${places ? `.${digits.slice(-places)}` : ''}`;
}
function fixedTime(n, places) {
  if (n !== 0 && Math.abs(n) < 0.5 * 10 ** -places) return n.toExponential(2);
  floatBits.setFloat64(0, Math.abs(n));
  const bits = floatBits.getBigUint64(0);
  const exponent = Number((bits >> 52n) & 2047n);
  const mantissa = (bits & ((1n << 52n) - 1n)) + (exponent ? 1n << 52n : 0n);
  const shift = (exponent ? exponent - 1023 - 52 : -1074) + places;
  const scaled = mantissa * 5n ** BigInt(places);
  let rounded;
  if (shift >= 0) rounded = scaled << BigInt(shift);
  else {
    const denominator = 1n << BigInt(-shift);
    rounded = scaled / denominator;
    const twice = 2n * (scaled % denominator);
    if (twice > denominator || (twice === denominator && rounded % 2n !== 0n)) rounded++;
  }
  return decimalText(rounded, places, n < 0);
}

/** Display-only formatting; coordinates, typed values and exact tables keep
 * their original precision. One unit is shared by each displayed row. */
export function timeSummaryFormat(values, columns = [], summary = {}) {
  const error = columns.some((c) => c.kind === 'mean') ? summary.mean_sampling?.standard_error : null;
  const magnitude = (v) => {
    if (!['duration', 'squared_duration'].includes(v?.kind)) return null;
    const n = timeValue(v);
    return n ? Number(`${n.n}e-${n.scale}`) : null;
  };
  const shown = [...values, error].filter(Boolean);
  if (!shown.some((v) => magnitude(v) !== null)) {
    const clocks = shown.filter((v) => v.kind === 'datetime').map(timeValue).filter(Boolean);
    if (clocks.length) {
      const ns = clocks.map((n) => n.n);
      const lo = ns.reduce((a,b) => a < b ? a : b), hi = ns.reduce((a,b) => a > b ? a : b);
      shown.push({ kind: 'duration', value: `${hi - lo}ns` });
    }
  }
  const durationValues = shown.filter((v) => magnitude(v) !== null);
  const squared = durationValues[0]?.kind === 'squared_duration';
  const power = squared ? 2 : 1;
  const magnitudes = durationValues.map(magnitude).map(Math.abs).filter((n) => n > 0 && Number.isFinite(n));
  const smallest = magnitudes.length ? Math.min(...magnitudes) : null;
  const [unit, base] = smallest === null ? ['s', 1000000000n]
    : units.find(([, n]) => smallest >= Number(n) ** power) ?? units.at(-1);
  const factor = Number(base) ** power;
  const moments = values.filter((_, i) => ['mean','sd'].includes(columns[i]?.kind)).map(magnitude).filter(Number.isFinite);
  const scale = Math.max(0, ...(moments.length ? moments : durationValues.map(magnitude).filter(Number.isFinite)).map((n) => Math.abs(n) / factor));
  const decimals = scale < 100 ? 2 : 0;
  const step = smallest === null ? 1n : 10n ** BigInt(Math.max(0, Math.min(9, Math.floor(Math.log10(factor) - decimals))));
  const value = (v, compact = false) => {
    const parsed = timeValue(v);
    if (!parsed) return 'unavailable';
    const amount = magnitude(v);
    if (amount !== null) {
      if (!Number.isFinite(amount)) return v.value;
      let text;
      if (parsed.scale === 0 && !Number.isSafeInteger(amount)) {
        const divisor = base ** BigInt(power);
        const magnitude = parsed.n < 0n ? -parsed.n : parsed.n;
        const places = compact && magnitude % divisor === 0n ? 0 : decimals;
        const scaled = magnitude * 10n ** BigInt(places);
        let rounded = scaled / divisor;
        const twice = 2n * (scaled % divisor);
        if (twice > divisor || (twice === divisor && rounded % 2n !== 0n)) rounded++;
        text = decimalText(rounded, places, parsed.n < 0n);
      } else {
        const n = amount / factor;
        const places = compact && Number.isInteger(n) ? 0 : decimals;
        text = fixedTime(n, places);
      }
      return `${text}${unit}${squared ? '^2' : ''}`;
    }
    if (v.kind !== 'datetime') return v.value;
    let whole = parsed.n / step, remainder = parsed.n % step;
    if (remainder < 0n) { whole--; remainder += step; }
    const rounded = (whole + (2n * remainder > step ? 1n : 0n)) * step;
    const text = calendarText('datetime', rounded);
    return /^\d{4}-/.test(text) && !text.startsWith('0000') ? text : v.value;
  };
  return {
    value,
    mean(v) {
      const text = value(v);
      const n = magnitude(error);
      return n !== null && n / factor >= 0.5 * 10 ** -decimals ? `${text} ± ${value(error)}` : text;
    },
  };
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
