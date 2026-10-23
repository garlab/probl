import { alignedBaseline, chartSeries, columnEstimate, compatibleBaselines, currencies, keyId, segments } from './chart-data.js';

const svgNS = 'http://www.w3.org/2000/svg';
const finite = (n) => typeof n === 'number' && Number.isFinite(n);
const plain = new Intl.NumberFormat('en', { maximumSignificantDigits: 7 });
const percent = new Intl.NumberFormat('en', { style: 'percent', maximumFractionDigits: 2 });
const textNumber = (n) => finite(n) ? plain.format(n) : 'unavailable';
const compactValues = (series) => series.deterministic && !series.report.summary_selection && !series.columns[series.primary].approximate;
const lineLabel = (series) => compactValues(series) ? 'Value' : series.columns[series.primary].label;
const baselineLabel = (series) => `${series.report.label}${series.columns[series.primary].approximate ? ` (${lineLabel(series)})` : ''}`;

function element(tag, attrs = {}, text) {
  const node = document.createElement(tag);
  for (const [name, value] of Object.entries(attrs)) node.setAttribute(name, value);
  if (text !== undefined) node.textContent = text;
  return node;
}
function svg(tag, attrs = {}, text) {
  const node = document.createElementNS(svgNS, tag);
  for (const [name, value] of Object.entries(attrs)) node.setAttribute(name, value);
  if (text !== undefined) node.textContent = text;
  return node;
}

function numberFormat(series, currency, compact = false) {
  return new Intl.NumberFormat('en', {
    ...(series.percent ? { style: 'percent' } : currency ? { style: 'currency', currency } : {}),
    ...(compact ? { notation: 'compact', maximumFractionDigits: 1 } : { maximumSignificantDigits: 7 }),
  });
}

function estimateNotes(q, fmt) {
  if (!q) return 'Estimate unavailable.';
  const notes = [];
  if (!q.complete) notes.push(`Incomplete; unresolved bounds ${q.bounds?.map((x) => finite(x) ? fmt.format(x) : x).join(' to ') ?? 'unavailable'}.`);
  if (q.sampling) {
    const s = q.sampling;
    notes.push(finite(s.standard_error) ? `Monte Carlo standard error ${fmt.format(s.standard_error)}.` : `Monte Carlo error ${s.status.replaceAll('_', ' ')}.`);
    if (s.confidence_interval) {
      const ci = s.confidence_interval;
      notes.push(`${percent.format(ci.level)} ${ci.method === 'wilson' ? 'Wilson' : ci.method} confidence interval ${fmt.format(ci.lower)} to ${fmt.format(ci.upper)}.`);
    }
  }
  return notes.join(' ');
}

function groupNotes(series, point, fmt) {
  const g = point.group;
  const quantity = columnEstimate(g, series.columns[series.primary]);
  const notes = [estimateNotes(quantity, fmt)];
  const approximation = g.summary?.approximation;
  if (approximation && series.columns.some((c) => c.approximate)) {
    notes.push(`Approximate quantiles: capacity ${approximation.capacity}; ${approximation.samples} samples; ${approximation.zeros} zeros counted exactly.`);
    if (approximation.rank_error_bound === null) notes.push('No rank-error bound is available.');
  }
  if (g.support) notes.push(`${g.support.contributing_runs} contributing runs; ${textNumber(g.support.effective_runs)} effective.`);
  if (g.complete === false) notes.push(`Unresolved share ${percent.format(g.unresolved_share)}.`);
  return notes.filter(Boolean).join(' ');
}

function reportNotes(series, run) {
  const notes = [];
  if (series.columns.some((c) => c.approximate)) notes.push('Quantile approximation is separate from Monte Carlo uncertainty. All percentiles, including Median, use the inverse CDF.');
  if (run.partial) notes.push(`Partial result: ${finite(run.failed_share) ? percent.format(run.failed_share) + ' of ' : ''}worlds/runs failed. See the failures in the text output.`);
  if (!run.unresolved.zero) notes.push('Some model weight is unresolved; estimates may be incomplete. Inspect each point’s bounds.');
  const { reach, visits } = series.report;
  notes.push(reach ? `Report reach ${percent.format(reach.share)}${finite(reach.standard_error) ? ` (standard error ${percent.format(reach.standard_error)})` : ''}.` : 'Report reach is unavailable.');
  notes.push(visits === 'per_visit' ? 'Every visit counts; repeated visits are not independent worlds.' : visits === 'per_key' ? 'At most one contribution per world and key.' : 'At most one contribution per world.');
  if (run.evidence) notes.push(`Conditioned/weighted results (${run.evidence.kind} evidence).`);
  if (run.integration_tolerance !== null) notes.push(`Numerical integration tolerance ${run.integration_tolerance}.`);
  return notes.join(' ');
}

/** Returns cleanup for resize observers. The text output is always retained. */
export function renderCharts(container, document, settings, onChange) {
  container.replaceChildren();
  const series = chartSeries(document);
  container.hidden = series.length === 0;
  if (!series.length) return () => {};
  container.append(element('h2', {}, 'Charts'));
  if (series.length < document.reports.length) {
    container.append(element('p', { class: 'chart-note' }, 'Other reports remain in the text output below. Charts support up to 1,000 ordered numeric/date keys per report and 20 reports.'));
  }
  const observers = [];
  for (const s of series) {
    const id = s.report.id;
    const prefix = `chart-${id}`;
    const card = element('section', { class: 'report-chart', 'data-report-id': id, 'aria-labelledby': `${prefix}-title` });
    card.append(element('h3', { id: `${prefix}-title` }, s.report.label));
    const candidates = compatibleBaselines(s, series);
    let choice = settings.reports[id] ?? { baseline: null, currency: '' };
    let baseline = candidates.find((c) => c.report.id === choice.baseline) ?? null;
    const controls = element('div', { class: 'chart-controls' });
    let currency = s.percent ? '' : choice.currency;
    const update = () => {
      choice = { baseline: baseline?.report.id ?? null, currency };
      onChange(id, choice);
      draw();
      inspect();
      table.replaceChildren();
      if (details.open) fillTable();
    };
    if (candidates.length) {
      const label = element('label', {}, 'Overlay (same units) ');
      const select = element('select', { class: 'chart-baseline', 'aria-label': `Baseline for ${s.report.label}`, title: 'Select only a baseline measured in the same units. Only matching keys are drawn.' });
      select.append(element('option', { value: '' }, 'None'));
      for (const c of candidates) select.append(element('option', { value: c.report.id }, `${c.report.id + 1} · ${c.report.label}`));
      select.value = baseline ? String(baseline.report.id) : '';
      select.addEventListener('change', () => { baseline = candidates.find((c) => String(c.report.id) === select.value) ?? null; update(); });
      label.append(select);
      controls.append(label);
    }
    if (!s.percent) {
      const label = element('label', {}, 'Values ');
      const select = element('select', { class: 'chart-currency', 'aria-label': `Value format for ${s.report.label}` });
      for (const code of currencies) select.append(element('option', { value: code }, code || 'Number'));
      select.value = currency;
      select.addEventListener('change', () => { currency = select.value; update(); });
      label.append(select);
      controls.append(label);
    }
    card.append(controls);
    const legend = element('p', { class: 'chart-legend', id: `${prefix}-legend` });
    card.append(legend);
    const plot = svg('svg', { class: 'chart-plot', role: 'img', 'aria-labelledby': `${prefix}-title ${prefix}-legend` });
    card.append(plot);
    const inspector = element('div', { class: 'chart-inspector' });
    const slider = element('input', { type: 'range', min: 0, max: s.points.length - 1, value: 0, step: 1, 'aria-label': `Inspect ${s.report.label} by ${s.report.key_label}`, class: 'chart-position' });
    const readout = element('div', { class: 'chart-readout', role: 'status', 'aria-live': 'polite', 'aria-atomic': 'true' });
    inspector.append(element('p', { class: 'chart-hint' }, 'Hover to inspect, or focus the slider and use arrow keys, Home or End.'), slider, readout);
    card.append(inspector);
    const explanation = s.kind === 'probability' ? 'Step curve: probability at each observed key. Monte Carlo error and confidence intervals are shown separately in the point details.'
      : s.deterministic ? 'One resolved value at each key. Deterministic values may still come from a partial or incomplete result.'
      : s.bands.length ? `Bands show the ${s.bands.map((b) => b.label).join(' and ')} outcome percentiles; the line shows ${lineLabel(s).toLowerCase()}. These are pointwise ranges across futures, not a single trajectory or Monte Carlo confidence bands.`
      : `The line shows ${lineLabel(s).toLowerCase()} at each key. Monte Carlo uncertainty is shown separately in the point details.`;
    card.append(element('p', { class: 'chart-note' }, explanation));
    card.append(element('p', { class: 'chart-note chart-qualifications' }, reportNotes(s, document.run)));
    const details = element('details', { class: 'chart-table' });
    details.append(element('summary', {}, 'Values and uncertainty'));
    const table = element('table');
    const tableWrap = element('div', { class: 'chart-table-scroll', tabindex: 0, 'aria-label': `Values for ${s.report.label}` });
    tableWrap.append(table);
    details.append(tableWrap);
    card.append(details);
    container.append(card);
    let marker;
    let xScale;
    let width;

    function draw() {
      width = Math.max(220, Math.floor(plot.getBoundingClientRect().width));
      const height = 250, left = 66, right = width - 16, top = 12, bottom = 204;
      const xs = [s.points[0].x, s.points.at(-1).x];
      const baselinePoints = alignedBaseline(s, baseline);
      const ys = s.points.flatMap((p) => [p.y, ...s.bands.flatMap((b) => [p.values[b.lo], p.values[b.hi]])]).concat(baselinePoints.map((p) => p.y)).filter(finite);
      let low = s.percent && s.kind === 'probability' ? 0 : Math.min(0, ...ys);
      let high = s.percent && s.kind === 'probability' ? 1 : Math.max(...ys);
      if (low === high) high = low + 1;
      // Scaling before subtracting also handles a finite domain whose span
      // overflows f64 (e.g. -1e308 through +1e308).
      const unit = Math.max(Math.abs(low), Math.abs(high), 1);
      const y = (v) => bottom - ((v / unit - low / unit) / (high / unit - low / unit)) * (bottom - top);
      const xUnit = Math.max(Math.abs(xs[0]), Math.abs(xs[1]), 1);
      xScale = (v) => xs[0] === xs[1] ? (left + right) / 2 : left + ((v / xUnit - xs[0] / xUnit) / (xs[1] / xUnit - xs[0] / xUnit)) * (right - left);
      const fmt = numberFormat(s, currency, true);
      plot.setAttribute('viewBox', `0 0 ${width} ${height}`);
      plot.replaceChildren();
      for (let i = 0; i <= 4; i++) {
        const value = (low / unit * (1 - i / 4) + high / unit * (i / 4)) * unit;
        plot.append(svg('line', { x1: left, x2: right, y1: y(value), y2: y(value), class: 'chart-grid' }));
        plot.append(svg('text', { x: left - 8, y: y(value) + 4, 'text-anchor': 'end' }, fmt.format(value)));
      }
      const ticks = [...new Set(width < 440 ? [0, s.points.length - 1] : [0, Math.floor((s.points.length - 1) / 2), s.points.length - 1])];
      for (const i of ticks) {
        const p = s.points[i];
        const label = p.key.kind === 'date' ? p.key.value : p.key.kind === 'prob' ? percent.format(p.x) : new Intl.NumberFormat('en', { notation: 'compact', maximumFractionDigits: 1 }).format(p.x);
        plot.append(svg('text', { x: xScale(p.x), y: bottom + 21, 'text-anchor': i === 0 ? 'start' : i === s.points.length - 1 ? 'end' : 'middle' }, label));
      }
      const maxLabel = Math.floor((right - left) / 6);
      const keyLabel = s.report.key_label.length > maxLabel ? `${s.report.key_label.slice(0, maxLabel - 1)}…` : s.report.key_label;
      plot.append(svg('text', { x: (left + right) / 2, y: height - 4, 'text-anchor': 'middle', class: 'chart-axis-label' }, keyLabel));
      if (s.kind === 'number' && !s.deterministic) {
        for (const [index, {lo, hi}] of s.bands.entries()) {
          for (const part of segments(s.points, (p) => [p.values[lo], p.values[hi]])) {
            if (part.length < 2) continue;
            const pairs = [...part.map((p) => [xScale(p.x), y(p.values[hi])]), ...part.toReversed().map((p) => [xScale(p.x), y(p.values[lo])])];
            plot.append(svg('path', { d: `M${pairs.map((p) => p.join(',')).join('L')}Z`, class: `chart-band ${index === 0 ? 'outer' : 'inner'}` }));
          }
        }
      }
      const line = (points, cls, step) => {
        for (const part of segments(points)) {
          const path = part.map((p, i) => i === 0 ? `M${xScale(p.x)},${y(p.y)}` : step ? `H${xScale(p.x)}V${y(p.y)}` : `L${xScale(p.x)},${y(p.y)}`).join('');
          plot.append(svg('path', { d: path, class: cls }));
          if (part.length === 1) plot.append(svg('circle', { cx: xScale(part[0].x), cy: y(part[0].y), r: 3, class: `${cls} point` }));
        }
      };
      line(s.points, 'chart-line', s.kind === 'probability');
      if (baseline) line(baselinePoints, 'chart-baseline-line', false);
      marker = svg('line', { y1: top, y2: bottom, class: 'chart-marker' });
      plot.append(marker);
      legend.replaceChildren(element('span', { class: 'legend-line' }, lineLabel(s)));
      if (s.bands.length && !s.deterministic) legend.append(element('span', { class: 'legend-band' }, s.bands.map((b) => b.label).join(' / ')));
      if (baseline) legend.append(element('span', { class: 'legend-baseline' }, baselineLabel(baseline)));
      inspect();
    }

    function inspect() {
      const point = s.points[Number(slider.value)];
      if (!point) return;
      const fmt = numberFormat(s, currency);
      const show = (n) => finite(n) ? fmt.format(n) : 'unavailable';
      const parts = [`${s.report.key_label}: ${point.key.label}`];
      parts.push(`${lineLabel(s)}: ${show(point.y)}`);
      if (s.report.summary_selection && s.kind !== 'probability') {
        s.columns.forEach((c, i) => { if (i !== s.primary) parts.push(`${c.label}: ${show(point.values[i])}`); });
      } else if (!s.deterministic) {
        for (const band of s.bands) parts.push(`${band.label}: ${show(point.values[band.lo])} to ${show(point.values[band.hi])}`);
      }
      const baselinePoint = baseline?.points.find((p) => keyId(p.key) === keyId(point.key));
      if (baseline) parts.push(`${baselineLabel(baseline)}: ${show(alignedBaseline(s, baseline)[Number(slider.value)].y)}`);
      slider.setAttribute('aria-valuetext', parts.join('; '));
      readout.replaceChildren(element('p', { class: 'chart-values' }, parts.join(' · ')), element('p', { class: 'chart-note' }, groupNotes(s, point, fmt)));
      if (baseline) readout.append(element('p', { class: 'chart-note' }, `${baseline.report.label}: ${baselinePoint ? groupNotes(baseline, baselinePoint, fmt) : 'No matching key.'} ${reportNotes(baseline, document.run)}`));
      readout.dataset.key = keyId(point.key);
      readout.dataset.value = finite(point.y) ? String(point.y) : '';
      marker?.setAttribute('x1', xScale(point.x));
      marker?.setAttribute('x2', xScale(point.x));
    }
    slider.addEventListener('input', inspect);
    plot.addEventListener('pointermove', (event) => {
      const bounds = plot.getBoundingClientRect();
      const x = (event.clientX - bounds.left) * width / bounds.width;
      let closest = 0;
      for (let i = 1; i < s.points.length; i++) if (Math.abs(xScale(s.points[i].x) - x) < Math.abs(xScale(s.points[closest].x) - x)) closest = i;
      slider.value = closest;
      inspect();
    });

    function fillTable() {
      if (table.childElementCount) return;
      table.append(element('caption', {}, s.report.label));
      const head = element('thead');
      const row = element('tr');
      const compact = compactValues(s);
      const columns = compact ? ['Value'] : s.columns.map((c) => c.label);
      for (const label of [s.report.key_label, ...columns, ...(baseline ? [baselineLabel(baseline)] : []), 'Uncertainty and support']) row.append(element('th', { scope: 'col' }, label));
      head.append(row);
      table.append(head);
      const body = element('tbody');
      const fmt = numberFormat(s, currency);
      const aligned = alignedBaseline(s, baseline);
      s.points.forEach((p, i) => {
        const row = element('tr', { 'data-group-id': p.group.id });
        row.append(element('th', { scope: 'row' }, p.key.label));
        const values = compact ? [p.y] : p.values;
        for (const value of [...values, ...(baseline ? [aligned[i].y] : [])]) row.append(element('td', {}, finite(value) ? fmt.format(value) : 'unavailable'));
        row.append(element('td', {}, groupNotes(s, p, fmt)));
        body.append(row);
      });
      table.append(body);
    }
    details.addEventListener('toggle', () => { if (details.open) fillTable(); });
    const observer = new ResizeObserver(draw);
    observer.observe(card);
    observers.push(observer);
    draw();
  }
  return () => observers.forEach((observer) => observer.disconnect());
}
