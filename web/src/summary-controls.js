import { quantileCapacity } from './chart-data.js';

// Compiler report IDs identify choices even before the first run, so invalid
// settings from a share link can be corrected without a successful execution.
export function renderSummaryControls(container, reports, settings, apply) {
  const open = container.open;
  container.replaceChildren();
  container.hidden = !reports?.length;
  if (container.hidden) return;
  const make = (tag, text) => {
    const el = document.createElement(tag);
    if (text !== undefined) el.textContent = text;
    return el;
  };
  container.append(make('summary', 'Report summaries'));
  container.append(make('p', 'Choose statistics and collection for each report. Exact collection retains populations; Median uses midpoint ties and q:0.5 uses the inverse CDF. Moments only saves memory in sample mode by retaining mean and SD. Probability reports keep their probability and sampling uncertainty.'));
  const quantileHelp = make('p', 'Approximate quantiles save memory in sample mode for unweighted numeric reports with one value per run and key. All percentiles, including Median, use the inverse CDF. Higher capacity uses more memory and usually improves percentile accuracy; it is not an error bound.');
  quantileHelp.id = 'quantile-collection-help';
  quantileHelp.hidden = true;
  const form = make('form');
  const fields = [];
  const updateHelp = () => { quantileHelp.hidden = !fields.some(({ collection }) => collection.value === 'quantiles'); };
  for (const report of reports.slice(0, 256)) {
    const row = make('div');
    row.className = 'summary-row';
    row.dataset.reportId = report.id;
    const name = make('label', `${report.id + 1} · ${report.label}`);
    const select = make('select');
    select.id = `summary-${report.id}`;
    select.setAttribute('aria-label', `Summary for ${report.label}`);
    name.htmlFor = select.id;
    for (const [value, label] of [['', 'Default'], ['forecast', '10–90% bands'], ['moments', 'Mean and SD'], ['custom', 'Custom columns']]) {
      const option = make('option', label);
      option.value = value;
      select.append(option);
    }
    const input = make('input');
    input.type = 'text';
    input.maxLength = 4096;
    input.placeholder = 'mean,sd,median,q:0.1,q:0.9';
    input.setAttribute('aria-label', `Custom summary for ${report.label}`);
    const value = settings.reports[report.id]?.summary ?? '';
    select.value = ['', 'forecast', 'moments'].includes(value) ? value : 'custom';
    input.value = select.value === 'custom' ? value : '';
    const change = () => { input.hidden = select.value !== 'custom'; input.required = !input.hidden; };
    select.addEventListener('change', change);
    change();
    const collection = make('select');
    collection.id = `collection-${report.id}`;
    collection.setAttribute('aria-label', `Collection for ${report.label}`);
    for (const [value, label] of [['exact', 'Exact collection'], ['moments', 'Moments only (sample mode)'], ['quantiles', 'Approximate quantiles (sample mode)']]) {
      const option = make('option', label);
      option.value = value;
      collection.append(option);
    }
    const saved = settings.reports[report.id]?.collection;
    const savedCapacity = quantileCapacity(saved);
    collection.value = savedCapacity !== null ? 'quantiles' : saved ?? 'exact';
    const capacityLabel = make('label', 'Capacity (128–4096) ');
    capacityLabel.className = 'summary-capacity';
    const capacity = make('input');
    Object.assign(capacity, { type: 'number', id: `capacity-${report.id}`, min: '128', max: '4096', step: '1', value: String(savedCapacity ?? 512) });
    capacity.setAttribute('aria-label', `Quantile capacity for ${report.label}`);
    capacity.setAttribute('aria-describedby', quantileHelp.id);
    capacityLabel.append(capacity);
    const changeCollection = () => {
      capacityLabel.hidden = collection.value !== 'quantiles';
      capacity.disabled = capacityLabel.hidden;
      capacity.required = !capacity.disabled;
      updateHelp();
    };
    collection.addEventListener('change', () => {
      if (collection.value === 'moments') { select.value = 'moments'; change(); }
      changeCollection();
    });
    row.append(name, select, collection, input, capacityLabel);
    fields.push({ id: report.id, select, input, collection, capacity });
    changeCollection();
    form.append(row);
  }
  const button = make('button', 'Apply and run');
  button.type = 'submit';
  form.append(quantileHelp, button);
  form.addEventListener('submit', (event) => {
    event.preventDefault();
    apply(Object.fromEntries(fields.map(({ id, select, input, collection, capacity }) => [id, {
      summary: select.value === 'custom' ? input.value.trim() : select.value,
      collection: collection.value === 'quantiles' ? `quantiles:${capacity.valueAsNumber}` : collection.value,
    }])));
  });
  container.append(form);
  container.open = open;
}
