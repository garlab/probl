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
  container.append(make('p', 'Choose displayed statistics for each report. Median uses midpoint ties; q:0.5 uses the weighted inverse CDF. Probability reports keep their probability and uncertainty.'));
  const form = make('form');
  const fields = [];
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
    row.append(name, select, input);
    fields.push({ id: report.id, select, input });
    form.append(row);
  }
  const button = make('button', 'Apply and run');
  button.type = 'submit';
  form.append(button);
  form.addEventListener('submit', (event) => {
    event.preventDefault();
    apply(Object.fromEntries(fields.map(({ id, select, input }) => [id, select.value === 'custom' ? input.value.trim() : select.value])));
  });
  container.append(form);
  container.open = open;
}
