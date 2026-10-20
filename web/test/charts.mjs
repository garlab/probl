// The real report schema, plus precision, gap and alignment edge cases which
// must not turn into invented observations in the browser.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { load } from '../src/probl.js';
import { alignedBaseline, chartSeries, chartSettings, compatibleBaselines, MAX_POINTS, readChartSettings, segments, seriesFor } from '../src/chart-data.js';

const engine = await load(await readFile(new URL('../dist/probl.wasm', import.meta.url)));
const run = (source) => engine.run({ source, today: '2026-10-19' }).reports;
const fixture = run(await readFile(new URL('report-json.probl', import.meta.url), 'utf8'));
const original = JSON.stringify(fixture);
const all = chartSeries(fixture);
assert.equal(all.length, 3);
assert.deepEqual(all[0].points.map((p) => p.y), [3.5, 3.5, 3.5, 3.5]);
assert.deepEqual(compatibleBaselines(all[0], all).map((s) => s.report.id), [2]);
assert.deepEqual(alignedBaseline(all[0], all[2]).map((p) => p.y), [0, 1, 2, 3]);
assert.equal(all[1].kind, 'probability');
assert.equal(all[1].percent, true);
assert.equal(JSON.stringify(fixture), original);
assert.deepEqual(chartSeries({ ...fixture, schema_version: 9 }), []);
assert.deepEqual(chartSeries(undefined), []);

const gapReport = structuredClone(all[0].report);
gapReport.groups[1].summary.quantiles.forEach((q) => { q.estimate = null; q.value = null; });
const gaps = seriesFor(gapReport);
assert.deepEqual(segments(gaps.points).map((part) => part.map((p) => p.x)), [[0], [2, 3]]);
const missing = structuredClone(all[2].report);
missing.groups.splice(1, 1);
const baseline = seriesFor(missing);
assert.deepEqual(alignedBaseline(all[0], baseline).map((p) => p.y), [0, null, 2, 3]);
assert.deepEqual(segments(alignedBaseline(all[0], baseline)).map((part) => part.length), [1, 2]);
const renamed = { ...baseline, report: { ...baseline.report, label: all[0].report.label } };
assert.equal(compatibleBaselines(all[0], [renamed])[0].report.id, 2, 'identity does not depend on labels');
assert.deepEqual(compatibleBaselines(all[0], [{ ...baseline, keyKind: 'date' }]), []);
assert.deepEqual(compatibleBaselines(all[0], [{ ...baseline, percent: true }]), []);
assert.deepEqual(compatibleBaselines(all[0], [all[1]]), []);

for (const source of [
  'report 1 by 9007199254740993',
  'report 9007199254740993 by 1',
  'for k in [1,1.0] { report 1 by k }',
  'report d6 by "January"',
  'report "red" by 1',
  'report {a: d6} by 1',
  'report date(2026,1,1) by 1',
  'if false {report 1 by 1}',
]) assert.deepEqual(chartSeries(run(source)), [], source);
const tooMany = structuredClone(all[0].report);
tooMany.groups = Array(MAX_POINTS + 1).fill(tooMany.groups[0]);
assert.equal(seriesFor(tooMany), null);
const duplicate = structuredClone(all[0].report);
duplicate.groups[1].key = duplicate.groups[0].key;
assert.equal(seriesFor(duplicate), null);

const dates = chartSeries(run(await readFile(new URL('../../examples/12_invoice_calendar.probl', import.meta.url), 'utf8')))[0];
assert.equal(dates.keyKind, 'date');
assert.deepEqual(dates.points.map((p) => p.y), [1200, 2400, 3600, 3600]);
assert.equal(dates.points[2].x - dates.points[1].x, 28, 'calendar axis uses UTC day distances');
const zero = chartSeries(run('for year in 0..2 { report 0 by year; report false by year }'));
assert.deepEqual(zero[0].points.map((p) => p.y), [0, 0, 0]);
assert.equal(zero[0].deterministic, true);
assert.equal(chartSeries(run('report d6 by 1'))[0].points.length, 1);
const partial = run('@on_error partial\nlet n ~ d2; for k in 0..2 { report 1/(n-1) by k }');
assert.equal(partial.run.partial, true);
assert.equal(chartSeries(partial)[0].points.length, 3);

assert.deepEqual(readChartSettings('bad'), { version: 1, reports: {} });
assert.deepEqual(chartSettings({ version: 2, reports: { 0: { baseline: 2 } } }).reports, {});
assert.deepEqual(chartSettings({ version: 1, reports: { 0: { baseline: 2, currency: 'USD' }, 1: { baseline: '2', currency: 'bad' }, bad: {} } }).reports,
  { 0: { baseline: 2, currency: 'USD' }, 1: { baseline: null, currency: '' } });
console.log('all passed: chart eligibility, typed coordinates, quantiles, gaps, baselines, partial results and settings');

const configure = (spec, source = 'for k in 0..2 {report one_of([0:50%,10:50%]) by k}') => engine.run({ source, report_summaries: { 0: spec } }).reports;
const forecast = chartSeries(configure('forecast'))[0];
assert.deepEqual(forecast.bands.map((b) => b.label), ['10–90%', '25–75%']);
assert.deepEqual(forecast.columns.map((c) => c.label), ['10%', '25%', 'Median', '75%', '90%']);
const moments = chartSeries(configure('moments'))[0];
assert.deepEqual(moments.columns.map((c) => c.label), ['Mean', 'SD']);
assert.deepEqual(moments.bands, []);
assert.deepEqual(moments.points.map((p) => p.y), [5, 5, 5]);
const tied = chartSeries(configure('q:0.5,median'))[0];
assert.deepEqual(tied.points[0].values, [0, 5]);
assert.equal(tied.points[0].y, 5, 'midpoint median remains distinct from the inverse CDF');
const reversed = chartSeries(configure('q:0.9,q:0.1,median'))[0];
assert.deepEqual(reversed.bands, [{lo:1,hi:0,label:'10–90%'}]);
assert.deepEqual(reversed.points[0].values, [10, 0, 5], 'table columns retain requested order');
const precise = chartSeries(configure('q:0.29,q:0.29000000000000004,q:1e-7,q:0,q:1'))[0];
assert.deepEqual(precise.columns.map((c) => c.label), ['29%', '29.000000000000004%', '0.00001%', '0%', '100%']);
const sd = chartSeries(configure('sd', 'for k in 0..2 {report k by k}'))[0];
assert.equal(sd.columns[sd.primary].label, 'SD');
assert.equal(sd.deterministic, false, 'an SD line cannot stand in for a value baseline');
assert.deepEqual(compatibleBaselines(sd, all), []);
assert.equal(chartSettings({version:1,reports:{0:{summary:'forecast'}}}).reports[0].summary, 'forecast');
console.log('all passed: configurable chart columns, bands, moments, median conventions and persistence');

const retained = engine.run({source:'@mode sample(runs:100,seed:42)\nfor year in 0..2 {let x~d6; report x by year}',report_collections:{0:'moments'}});
assert.equal(retained.reports.reports[0].collection, 'moments');
assert.equal(chartSeries(retained.reports)[0].columns[0].kind, 'mean');
assert.deepEqual(retained.reports.reports[0].groups[0].summary.quantiles, []);
assert.equal(chartSettings({version:1,reports:{0:{collection:'moments'},1:{collection:'bad'}}}).reports[0].collection, 'moments');
assert.equal(chartSettings({version:1,reports:{0:{collection:'moments'},1:{collection:'bad'}}}).reports[1].collection, undefined);
