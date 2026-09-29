// The playground in real browsers, headless: every example prints what
// `probl run` prints; stopping, limits, errors in the editor and share
// links work; and so do completion, descriptions on hover, going to a
// definition, the reference and the guide. Needs `npm run build`, `cargo
// build --release -p probl-cli`, and Chrome or Firefox where macOS puts
// them (or PROBL_CHROME and PROBL_FIREFOX).
//
//   node test/page.mjs            # Chrome
//   node test/page.mjs firefox    # Firefox
//
// It tests web/dist, served here, or the page at PROBL_URL, such as a
// deployment.

import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import * as esbuild from 'esbuild';
import puppeteer from 'puppeteer-core';

const root = fileURLToPath(new URL('../..', import.meta.url));
const firefox = process.argv.includes('firefox');
const browserPath = firefox
  ? process.env.PROBL_FIREFOX ?? '/Applications/Firefox.app/Contents/MacOS/firefox'
  : process.env.PROBL_CHROME ?? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';

const server = process.env.PROBL_URL ? null : await esbuild.context({});
const url = server
  ? `http://127.0.0.1:${(await server.serve({ servedir: `${root}web/dist`, port: 0 })).port}/`
  : new URL('./', process.env.PROBL_URL).href;
const browser = await puppeteer.launch({
  browser: firefox ? 'firefox' : 'chrome',
  executablePath: browserPath,
  headless: true,
});

let failures = 0;
function expect(ok, what, detail = '') {
  console.log(`${ok ? 'ok  ' : 'FAIL'}  ${what}${ok || !detail ? '' : `\n${detail}`}`);
  failures += !ok;
}

async function open(address = url) {
  const page = await browser.newPage();
  await page.goto(address);
  await page.waitForSelector('body[data-ready="true"]', { timeout: 30_000 });
  return page;
}

/** Press a button. (A DOM click: Firefox's WebDriver clicks don't always
 * reach a page that just loaded.) */
async function press(page, id) {
  await page.evaluate((id) => document.getElementById(id).click(), id);
}

/** Whether `test` becomes true in the page, within `timeout` ms. */
function becomes(page, test, arg, timeout = 5000) {
  return page.waitForFunction(test, { timeout, polling: 50 }, arg).then(
    () => true,
    () => false,
  );
}

/** Run the editor's program, or what `start` starts, and wait for it to
 * finish. */
async function run(page, start = () => press(page, 'run')) {
  const before = await page.evaluate(() => window.playground.finished());
  await start();
  await page.waitForFunction((n) => window.playground.finished() > n, { timeout: 120_000, polling: 50 }, before);
  return page.evaluate(() => ({
    result: document.getElementById('result').textContent,
    status: document.getElementById('status').textContent,
    error: document.getElementById('result').className === 'error',
  }));
}

try {
  const page = await open();

  // Every example, as the command line prints it.
  const examples = await page.evaluate(() => window.playground.examples.map((e) => e.name));
  for (const name of examples) {
    await page.select('#examples', name);
    const start = performance.now();
    const { result, status } = await run(page);
    const time = ((performance.now() - start) / 1000).toFixed(2);
    const expected = execFileSync(`${root}target/release/probl`, ['run', `examples/${name}.probl`], {
      cwd: root,
      encoding: 'utf8',
    });
    expect(`${result}\n` === expected, `${name} prints what \`probl run\` prints (${time} s; ${status})`, result);
  }

  // Stopping a long run, and running again after.
  await page.evaluate(() => window.playground.setSource('@mode sample(runs: 100_000_000, seed: 1)\nreport d6 > 3'));
  await press(page, 'run');
  const progressed = /Running… [\d,]+ of 100,000,000 runs/;
  await becomes(page, (re) => new RegExp(re).test(document.getElementById('status').textContent), progressed.source, 10_000);
  const progress = await page.$eval('#status', (e) => e.textContent);
  expect(progressed.test(progress), 'a long run shows its progress', progress);
  await press(page, 'stop');
  const stopped = await page.$eval('#status', (e) => e.textContent);
  expect(stopped === 'Stopped.', 'Stop stops it', stopped);
  await page.evaluate(() => window.playground.setSource('report d6 > 4'));
  let answer = await run(page);
  expect(answer.result.includes('33.33%'), 'the next run works', answer.result);

  // Calls nested as deep as the playground allows, and deeper.
  const deep = (n) =>
    `fn f(n: int) -> int {\n  if n == 0 { return 0 }\n  let rest = f(n - 1)\n  return rest + 1\n}\nreport f(${n})`;
  await page.evaluate((src) => window.playground.setSource(src), deep(149));
  answer = await run(page);
  expect(answer.result.includes('f(149)    149'), 'calls nest 149 deep', answer.result);
  await page.evaluate((src) => window.playground.setSource(src), deep(200));
  answer = await run(page);
  expect(
    answer.error && answer.result.includes('calls are nested more than 150 deep'),
    'deeper is a limit, not a crash',
    answer.result,
  );

  // Errors show in the editor as the program is edited.
  await page.evaluate(() => window.playground.setSource('let x = 1\nreport y'));
  const marked = await page
    .waitForSelector('.cm-lintRange-error', { timeout: 5000 })
    .then(() => true)
    .catch(() => false);
  expect(marked, 'a compile error is underlined');

  // A share link opens the same program.
  const program = 'let x ~ 2d6\nreport x >= 10 as "ten or more"';
  await page.evaluate((src) => window.playground.setSource(src), program);
  await press(page, 'share');
  await page.waitForFunction(() => location.hash.startsWith('#code='), { timeout: 5000 });
  const link = await page.evaluate(() => location.href);
  const other = await open(link);
  const opened = await other.evaluate(() => window.playground.getSource());
  expect(opened === program, 'a share link opens the same program', opened);

  // Before a run, the output pane says how to run the program; after, long
  // reports wrap to fit it.
  const fresh = await open();
  const hinted = await fresh.evaluate(() => document.getElementById('empty').offsetParent !== null);
  expect(hinted, 'before a run, the output pane says how to run the program');
  const loading = await fresh.evaluate(async () => {
    const page = await (await fetch('index.html')).text();
    const html = new DOMParser().parseFromString(page, 'text/html');
    return {
      before: [html.getElementById('status').textContent, html.getElementById('run').disabled],
      after: [document.getElementById('status').textContent, document.getElementById('run').disabled],
    };
  });
  expect(
    loading.before[0].startsWith('Loading') && loading.before[1] && loading.after[0] === '' && !loading.after[1],
    'while the engine loads, the page says so and Run waits',
    JSON.stringify(loading),
  );
  await fresh.select('#examples', '01_tour');
  await run(fresh);
  const fitted = await fresh.evaluate(() => {
    const result = document.getElementById('result');
    return {
      hint: document.getElementById('empty').offsetParent !== null,
      scroll: result.scrollWidth,
      width: result.clientWidth,
    };
  });
  expect(!fitted.hint && fitted.scroll <= fitted.width, 'long reports wrap to fit the pane', JSON.stringify(fitted));
  // Wrapping leaves each report's first line where it was: at the left.
  await fresh.select('#examples', '10_quantum_key');
  await run(fresh);
  const shifted = await fresh.evaluate(() => {
    const result = document.getElementById('result');
    const left = result.getBoundingClientRect().left + parseFloat(getComputedStyle(result).paddingLeft);
    return [...result.querySelectorAll('.line')]
      .filter((line) => line.textContent.trim() !== '')
      .map((line) => {
        const texts = document.createTreeWalker(line, NodeFilter.SHOW_TEXT);
        let text = texts.nextNode();
        while (text.length === 0) text = texts.nextNode();
        const range = document.createRange();
        range.setStart(text, 0);
        range.setEnd(text, 1);
        return [Math.round(range.getBoundingClientRect().left - left), line.textContent.slice(0, 30)];
      })
      .filter(([offset]) => Math.abs(offset) > 1);
  });
  expect(shifted.length === 0, 'and each report starts at the left of the pane', JSON.stringify(shifted));

  // Editing an example: the menu and the address stop naming it, so that
  // reloading keeps the edits.
  await fresh.select('#examples', '02_craps');
  const named = await fresh.evaluate(() => `${document.getElementById('examples').value} ${location.hash}`);
  expect(named === '02_craps #example=02_craps', 'an example is named in the menu and the address', named);
  await fresh.evaluate(() => {
    const { view } = window.playground;
    view.dispatch({ selection: { anchor: view.state.doc.length } });
    view.focus();
  });
  await fresh.keyboard.type('\nreport d4');
  const unnamed = await becomes(fresh, () => location.hash === '' && document.getElementById('examples').value === '');
  expect(unnamed, 'once it’s edited, neither names it');
  const edits = await fresh.evaluate(() => window.playground.getSource());
  await fresh.reload();
  await fresh.waitForSelector('body[data-ready="true"]', { timeout: 30_000 });
  const kept = await fresh.evaluate(() => window.playground.getSource());
  expect(kept === edits && kept.includes('d4'), 'reloading keeps the edits', JSON.stringify(kept.slice(-40)));

  // Replacing a program of your own says how to get it back, and undo does.
  await fresh.evaluate(() => window.playground.setSource('# mine\nreport d6'));
  await fresh.select('#examples', '03_rpg_duel');
  const replaced = await fresh.$eval('#status', (e) => e.textContent);
  expect(replaced.includes('brings it back'), 'replacing your own program says how to get it back', replaced);
  await fresh.evaluate(() => window.playground.view.focus());
  const mod = process.platform === 'darwin' ? 'Meta' : 'Control';
  await fresh.keyboard.down(mod);
  await fresh.keyboard.press('z');
  await fresh.keyboard.up(mod);
  const undone = await fresh.evaluate(() => window.playground.getSource());
  expect(undone === '# mine\nreport d6', 'and undo does', undone);

  // ── What the editor knows about the program ──
  const editor = await open();
  const source = [
    '@mode sample(runs: 1000, seed: 1)',
    'enum Plan { Free, Paid }',
    '',
    '# How often visitors sign up.',
    'let rate ~ beta(2, 40)   # a few percent, probably',
    'let visitors = 250',
    '',
    '# Sign-ups over some days: a binomial count.',
    'fn signups(days: int) -> int {',
    '  let n ~ binomial(days * visitors, rate)',
    '  return n',
    '}',
    'report rate',
    'report signups(14)',
    'type Week = { visits: int, conversions: int }',
    'let first = Week { visits: 1750, conversions: 60 }',
    'report first.conversions',
  ].join('\n');
  await editor.evaluate((src) => window.playground.setSource(src), source);
  /** Where the `n`th `needle` is in the program (from 0), plus `at`. */
  const offset = (needle, n = 0, at = 0) => {
    let i = -1;
    for (let k = 0; k <= n; k += 1) i = source.indexOf(needle, i + 1);
    return i + at;
  };
  const cursor = (pos) =>
    editor.evaluate((pos) => {
      window.playground.view.dispatch({ selection: { anchor: pos } });
      window.playground.view.focus();
    }, pos);
  const selected = () =>
    editor.evaluate(() => {
      const { state } = window.playground.view;
      const { from, to } = state.selection.main;
      return `${state.sliceDoc(from, to)} on line ${state.doc.lineAt(from).number}`;
    });
  const texts = (selector) =>
    editor.evaluate((selector) => [...document.querySelectorAll(selector)].map((e) => e.textContent), selector);
  // A mouse event over the character at `pos`, sent to what's there as the
  // browser sends it.
  const mouse = (type, pos, keys = {}) =>
    editor.evaluate(
      (type, pos, keys) => {
        const { view } = window.playground;
        const a = view.coordsAtPos(pos, 1);
        const b = view.coordsAtPos(pos + 1, -1);
        const x = (a.left + b.left) / 2;
        const y = (a.top + a.bottom) / 2;
        const init = { clientX: x, clientY: y, bubbles: true, cancelable: true, button: 0, ...keys };
        document.elementFromPoint(x, y).dispatchEvent(new MouseEvent(type, init));
      },
      type,
      pos,
      keys,
    );

  // The name under the cursor, and its other uses, once the checker has
  // found them.
  await cursor(offset('rate)', 0, 2));
  await becomes(editor, () => document.querySelectorAll('.cm-probl-same').length === 3);
  const same = await texts('.cm-probl-same');
  expect(same.join() === 'rate,rate,rate', 'the name under the cursor is highlighted where it’s used', same.join());

  // Hovering: a name of the program's, with its comments; a built-in; a
  // keyword.
  const unhover = () =>
    editor.evaluate(() =>
      window.playground.view.dom.dispatchEvent(new MouseEvent('mouseleave', { relatedTarget: document.body })),
    );
  async function hover(pos, timeout = 5000) {
    await unhover();
    await mouse('mousemove', pos);
    await becomes(editor, () => document.querySelector('.cm-probl-card'), undefined, timeout);
    return editor.evaluate(() => {
      const card = document.querySelector('.cm-probl-card');
      const part = (name) => card?.querySelector(`.cm-probl-${name}`)?.textContent ?? null;
      return { code: part('signature'), what: part('what'), doc: part('doc') };
    });
  }
  let card = await hover(offset('report rate', 0, 8));
  expect(
    card.code === 'let rate ~ beta(2, 40)' &&
      card.what === 'variable, line 5' &&
      card.doc === 'How often visitors sign up.\na few percent, probably',
    'hovering a variable shows its declaration and comments',
    JSON.stringify(card),
  );
  card = await hover(offset('signups(14)', 0, 2));
  expect(
    card.code === 'fn signups(days: int) -> int' && card.what === 'function, line 9' && card.doc === 'Sign-ups over some days: a binomial count.',
    'hovering a function shows its signature',
    JSON.stringify(card),
  );
  card = await hover(offset('binomial(', 0, 2));
  expect(card.code?.startsWith('binomial(') && card.doc?.length > 20, 'hovering a built-in documents it', JSON.stringify(card));
  card = await hover(offset('report rate', 0, 2));
  expect(card.code?.includes('report') && card.doc?.length > 20, 'hovering a keyword documents it', JSON.stringify(card));
  card = await hover(offset('binomial count', 0, 2), 1000);
  expect(card.code === null, 'words in comments aren’t described', JSON.stringify(card));

  await unhover();

  // Going to a definition: F12, and Cmd-click or Ctrl-click, with the name
  // underlined as a link while the key is down.
  await cursor(offset('signups(14)', 0, 3));
  await editor.keyboard.press('F12');
  let went = await selected();
  expect(went === 'signups on line 9', 'F12 goes to the definition', went);
  await mouse('mousemove', offset('rate)', 0, 1), { metaKey: true });
  const underlined = await texts('.cm-probl-link');
  expect(underlined.join() === 'rate', 'holding Cmd or Ctrl underlines a name', underlined.join());
  await mouse('mousedown', offset('rate)', 0, 1), { metaKey: true });
  went = await selected();
  expect(went === 'rate on line 5', 'Cmd-click goes to the definition', went);
  await mouse('mousedown', offset('visitors,', 0, 1), { ctrlKey: true });
  went = await selected();
  expect(went === 'visitors on line 6', 'so does Ctrl-click', went);
  await mouse('mousedown', offset('first.conversions', 0, 7), { metaKey: true });
  went = await selected();
  expect(went === 'conversions on line 15', 'a field goes to its record', went);
  await editor.evaluate(() =>
    window.playground.view.contentDOM.dispatchEvent(new KeyboardEvent('keyup', { key: 'Meta', bubbles: true })),
  );
  expect((await texts('.cm-probl-link')).length === 0, 'letting go of the key removes the underline');

  // A built-in's definition is its entry in the reference.
  await mouse('mousemove', offset('binomial(', 0, 1), { metaKey: true });
  const builtinLink = await texts('.cm-probl-link');
  expect(builtinLink.join() === 'binomial', 'holding Cmd or Ctrl underlines a built-in too', builtinLink.join());
  await cursor(offset('binomial(', 0, 2));
  await editor.keyboard.press('F12');
  const entry = await editor.evaluate(() => ({
    tab: document.querySelector('[role="tab"][aria-selected="true"]').dataset.pane,
    current: document.querySelector('#entries .entry.current')?.dataset.names,
    visible: document.querySelector('#entries .entry.current')?.offsetParent !== null,
  }));
  expect(
    entry.tab === 'reference' && entry.current === 'binomial' && entry.visible,
    'F12 on a built-in shows its entry in the reference',
    JSON.stringify(entry),
  );
  await editor.evaluate(() => document.querySelector('[data-pane="output"]').click());

  // F2 selects a name everywhere it's used, so that typing renames it.
  await cursor(offset('report rate', 0, 8));
  await editor.keyboard.press('F2');
  const uses = await editor.evaluate(() => {
    const { state } = window.playground.view;
    return state.selection.ranges.map((r) => `${state.sliceDoc(r.from, r.to)}@${state.doc.lineAt(r.from).number}`);
  });
  expect(uses.join() === 'rate@5,rate@10,rate@13', 'F2 selects a name wherever it’s used', uses.join());
  await editor.keyboard.type('share');
  const renamed = await editor.evaluate(() => window.playground.getSource());
  expect(
    (renamed.match(/\bshare\b/g) ?? []).length === 3 && !/\brate\b/.test(renamed.replace(/#.*$/gm, '')),
    'and typing renames it',
    renamed,
  );
  await editor.evaluate((src) => window.playground.setSource(src), source);

  // Completion, as you type: the program's names, the built-ins with their
  // documentation, and an enum's variants.
  const completions = async (typed) => {
    await editor.keyboard.type(typed);
    await becomes(editor, () => document.querySelector('.cm-tooltip-autocomplete li'));
    return texts('.cm-tooltip-autocomplete li .cm-completionLabel');
  };
  await cursor(source.length);
  let offered = await completions('\nreport visi');
  let details = await texts('.cm-tooltip-autocomplete li .cm-completionDetail');
  expect(
    offered[0] === 'visitors' && details[0] === '= 250',
    'completion offers the program’s names, with what they are',
    `${offered.join(', ')}\n${details.join(', ')}`,
  );
  // (Keys reach a completion list after it's been open 75 ms.)
  await new Promise((resolve) => setTimeout(resolve, 150));
  await editor.keyboard.press('Enter');
  let line = await editor.evaluate(() => window.playground.view.state.doc.lineAt(window.playground.view.state.selection.main.head).text);
  expect(line === 'report visitors', 'Enter takes the completion', line);
  offered = await completions('\nlet k ~ binom');
  details = await texts('.cm-tooltip-autocomplete li .cm-completionDetail');
  await becomes(editor, () => document.querySelector('.cm-completionInfo .cm-probl-card'));
  const info = await texts('.cm-completionInfo .cm-probl-signature');
  expect(
    offered[0] === 'binomial' && details[0]?.startsWith('(n: int') && info[0]?.startsWith('binomial('),
    'completion offers the built-ins, with their documentation',
    `${offered.join(', ')}\n${details.join(', ')}\n${info.join()}`,
  );
  await editor.keyboard.press('Escape');
  offered = await completions('\nlet p = Plan.');
  expect(offered.join() === 'Free,Paid', 'after an enum’s name, completion offers its variants', offered.join(', '));
  await editor.keyboard.press('Escape');
  await editor.keyboard.type('\n# the rate');
  await new Promise((resolve) => setTimeout(resolve, 400));
  offered = await texts('.cm-tooltip-autocomplete li');
  expect(offered.length === 0, 'there’s no completion in a comment', offered.join(', '));

  // Constants are documented names, and a program can still hide them.
  const math = 'report sin(pi / 2)\nreport euler_gamma';
  await editor.evaluate((src) => window.playground.setSource(src), math);
  card = await hover(math.indexOf('pi') + 1);
  expect(card.code === 'pi = 3.141592653589793', 'hovering a constant shows its value', JSON.stringify(card));
  await unhover();
  await cursor(math.indexOf('pi') + 1);
  await editor.keyboard.press('F12');
  const constantEntry = await editor.evaluate(() => ({
    tab: document.querySelector('[role="tab"][aria-selected="true"]').dataset.pane,
    current: document.querySelector('#entries .entry.current')?.dataset.names,
  }));
  expect(
    constantEntry.tab === 'reference' && constantEntry.current === 'pi',
    'F12 on a constant opens its reference entry',
    JSON.stringify(constantEntry),
  );
  await editor.evaluate(() => document.querySelector('[data-pane="output"]').click());
  await cursor(math.length);
  offered = await completions('\nreport euler_');
  details = await texts('.cm-tooltip-autocomplete li .cm-completionDetail');
  expect(
    offered[0] === 'euler_gamma' && details[0] === '0.5772156649015329',
    'completion offers constants with their values',
    `${offered.join(', ')}\n${details.join(', ')}`,
  );
  await editor.keyboard.press('Escape');
  const shadowed = 'let pi = 3\nreport pi';
  await editor.evaluate((src) => window.playground.setSource(src), shadowed);
  await cursor(shadowed.lastIndexOf('pi') + 1);
  await becomes(editor, () => document.querySelectorAll('.cm-probl-same').length === 2);
  card = await hover(shadowed.lastIndexOf('pi') + 1);
  expect(card.code === 'let pi = 3', 'a local name hides a constant on hover', JSON.stringify(card));
  await unhover();
  await editor.keyboard.press('F12');
  went = await selected();
  expect(went === 'pi on line 1', 'a local constant name goes to its declaration', went);

  // Typing a program as a person would, faster than the checker checks.
  const typed = () => editor.evaluate(() => window.playground.getSource());
  await editor.evaluate(() => window.playground.setSource(''));
  await cursor(0);
  offered = await completions('let quota = 3\nreport quo');
  expect(offered[0] === 'quota', 'completion knows a name declared a moment ago', offered.join(', '));
  await editor.keyboard.type('ta');
  await becomes(editor, () => document.querySelector('.cm-tooltip-autocomplete li'));
  await new Promise((resolve) => setTimeout(resolve, 150));
  await editor.keyboard.press('Enter');
  let text = await typed();
  expect(text === 'let quota = 3\nreport quota\n', 'Enter after a whole name starts a new line', JSON.stringify(text));
  await editor.keyboard.type('let enumer');
  await new Promise((resolve) => setTimeout(resolve, 400));
  offered = await texts('.cm-tooltip-autocomplete li');
  expect(offered.length === 0, 'there’s no completion while naming a variable', offered.join(', '));
  // Blocks indent, and their closing braces line up.
  await editor.evaluate(() => window.playground.setSource(''));
  await cursor(0);
  await editor.keyboard.type('fn f(x: int) -> int {\nif x > 1 {\nreturn x');
  text = await typed();
  expect(
    text === 'fn f(x: int) -> int {\n  if x > 1 {\n    return x\n  }\n}',
    'blocks indent as you type',
    JSON.stringify(text),
  );
  await editor.keyboard.press('Escape');
  const indented = 'if 50% {\n  report 1\n  ';
  await editor.evaluate((src) => window.playground.setSource(src), indented);
  await cursor(indented.length);
  await editor.keyboard.type('} else {\nreport 2');
  text = await typed();
  expect(
    text === 'if 50% {\n  report 1\n} else {\n  report 2\n}',
    'a closing brace goes back out to its block’s line',
    JSON.stringify(text),
  );
  await editor.keyboard.press('Escape');

  // A replaced program's names don't linger, even while the new one
  // doesn't parse.
  await editor.evaluate(() => window.playground.setSource('let a = 1\nreport a'));
  await cursor(17);
  await becomes(editor, () => document.querySelectorAll('.cm-probl-same').length === 2);
  // (Twice: a name forgotten at the first edit stays forgotten at the next.)
  await editor.evaluate(() => window.playground.setSource('let b = ('));
  await editor.evaluate(() => window.playground.setSource('let c = ('));
  await cursor(5);
  await new Promise((resolve) => setTimeout(resolve, 700));
  const lingering = await texts('.cm-probl-same');
  expect(lingering.length === 0, 'a replaced program’s names don’t linger', lingering.join(', '));

  // While a call's arguments are typed, its signature shows, with the
  // parameter being typed in bold.
  const own = 'fn total(days: int, rate: float) -> float {\n  return days * rate\n}\n';
  await editor.evaluate((src) => window.playground.setSource(src), own);
  await cursor(own.length);
  const signatureHelp = async (expected) => {
    await becomes(editor, (shown) => !!document.querySelector('.cm-probl-signature-help') === shown, expected, 2000);
    return editor.evaluate(() => {
      const help = document.querySelector('.cm-probl-signature-help');
      if (!help) return null;
      return { signature: help.querySelector('.cm-probl-signature').textContent, current: help.querySelector('b')?.textContent };
    });
  };
  await editor.keyboard.type('let k ~ binomial(');
  let help = await signatureHelp(true);
  expect(
    help?.signature.startsWith('binomial(') && help.current === 'n: int',
    'typing a call shows its signature',
    JSON.stringify(help),
  );
  await editor.keyboard.type('10, ');
  help = await signatureHelp(true);
  expect(help?.current === 'p: prob', 'which follows the argument being typed', JSON.stringify(help));
  await editor.keyboard.type('50%)');
  help = await signatureHelp(false);
  expect(help === null, 'and goes when the call is closed', JSON.stringify(help));
  await editor.keyboard.type('\nreport total(1, ');
  help = await signatureHelp(true);
  expect(
    help?.signature === 'total(days: int, rate: float) -> float' && help.current === 'rate: float',
    'the program’s own functions show theirs',
    JSON.stringify(help),
  );
  await editor.keyboard.press('Escape');
  help = await signatureHelp(false);
  expect(help === null, 'Escape hides it', JSON.stringify(help));

  // The tabs: each shows its pane alone.
  const showTab = (name) => editor.evaluate((name) => document.querySelector(`[data-pane="${name}"]`).click(), name);
  await showTab('reference');
  const layout = await editor.evaluate(() => ({
    output: getComputedStyle(document.getElementById('output')).display,
    gap: document.getElementById('reference').getBoundingClientRect().top - document.querySelector('.tabs').getBoundingClientRect().bottom,
  }));
  expect(layout.output === 'none' && Math.abs(layout.gap) < 1, 'a tab shows its pane alone', JSON.stringify(layout));

  // The tabs can be used from the keyboard: the arrow keys, Home and End.
  await editor.evaluate(() => document.querySelector('[data-pane="reference"]').focus());
  const tabbed = [];
  for (const key of ['ArrowRight', 'ArrowLeft', 'Home', 'End']) {
    await editor.keyboard.press(key);
    tabbed.push(
      await editor.evaluate(() => {
        const selected = document.querySelector('[role="tab"][aria-selected="true"]');
        const reachable = [...document.querySelectorAll('[role="tab"]')].filter((t) => t.tabIndex === 0);
        return document.activeElement === selected && reachable.length === 1 ? selected.dataset.pane : 'lost';
      }),
    );
  }
  expect(tabbed.join() === 'output,reference,output,reference', 'the tabs work from the keyboard', tabbed.join());

  // The divider between the editor and the side panel: dragged, or moved
  // with the arrow keys, and reset by a double click.
  const share = () =>
    editor.evaluate(() => {
      const main = document.querySelector('main').getBoundingClientRect();
      return Math.round((document.getElementById('editor').getBoundingClientRect().width / main.width) * 100);
    });
  const before = await share();
  await editor.evaluate(() => document.getElementById('divider').focus());
  await editor.keyboard.press('ArrowRight');
  await editor.keyboard.press('ArrowRight');
  const keyed = await share();
  expect(keyed - before >= 9 && keyed - before <= 11, 'the arrow keys move the divider', `${before}% to ${keyed}%`);
  const bar = await editor.evaluate(() => {
    const r = document.getElementById('divider').getBoundingClientRect();
    const main = document.querySelector('main').getBoundingClientRect();
    return { x: r.left + r.width / 2, y: r.top + r.height / 2, left: main.left, width: main.width };
  });
  await editor.mouse.move(bar.x, bar.y);
  await editor.mouse.down();
  await editor.mouse.move(bar.left + bar.width * 0.3, bar.y, { steps: 5 });
  await editor.mouse.up();
  const dragged = await share();
  expect(Math.abs(dragged - 30) <= 2, 'dragging moves the divider', `${dragged}%`);
  await editor.mouse.move(bar.left + bar.width * 0.3, bar.y);
  await editor.evaluate(() => document.getElementById('divider').dispatchEvent(new MouseEvent('dblclick', { bubbles: true })));
  const reset = await share();
  expect(Math.abs(reset - before) <= 1, 'a double click puts it back', `${reset}%`);

  // The reference, searched.
  const found = await editor.evaluate(() => {
    const search = document.getElementById('search');
    search.value = 'binomial';
    search.dispatchEvent(new Event('input'));
    const entries = [...document.querySelectorAll('#entries .entry')];
    const shown = entries.filter((e) => e.offsetParent !== null);
    const signatures = entries.map((e) => e.querySelector('.signature').textContent);
    return {
      all: entries.length,
      distinct: new Set(signatures).size,
      shown: shown.map((e) => e.querySelector('.signature').textContent),
      matching: shown.every((e) => e.dataset.search.includes('binomial')),
    };
  });
  expect(
    found.matching && found.shown.some((s) => s.startsWith('binomial(')) && found.shown.length < found.all / 4,
    'searching the reference shows what matches',
    JSON.stringify(found),
  );
  expect(found.distinct === found.all, 'the reference has each entry once', `${found.distinct} of ${found.all}`);

  // The guide: its programs run in the editor, and its links open the
  // examples.
  await showTab('guide');
  const guided = await becomes(editor, () => document.querySelectorAll('#guide button.try').length >= 10, undefined, 10_000);
  expect(guided, 'the guide loads, with programs to run');
  const address = await editor.evaluate(() => location.href);
  await editor.evaluate(() => document.querySelector('#guide .contents a[href="#5-evidence"]').click());
  const jumped = await editor.evaluate(() => ({
    offset: Math.round(
      document.getElementById('5-evidence').getBoundingClientRect().top -
        document.getElementById('guide').getBoundingClientRect().top,
    ),
    href: location.href,
  }));
  expect(
    Math.abs(jumped.offset) < 5 && jumped.href === address,
    'its contents lead to its sections, and leave the address alone',
    JSON.stringify(jumped),
  );
  const snippet = await editor.evaluate(() => document.querySelector('#guide button.try').previousElementSibling.textContent);
  answer = await run(editor, () => editor.evaluate(() => document.querySelector('#guide button.try').click()));
  const ran = await editor.evaluate(() => ({
    source: window.playground.getSource(),
    output: !document.getElementById('output').hidden,
  }));
  expect(
    ran.source === snippet && ran.output && !answer.error && answer.result.length > 0,
    'a program in the guide runs in the editor',
    `${ran.source}\n---\n${answer.result}`,
  );
  await showTab('guide');
  await editor.evaluate(() => document.querySelector('#guide a[href="#example=02_craps"]').click());
  const craps = await editor.evaluate(() => window.playground.examples.find((e) => e.name === '02_craps').source);
  const linked = await becomes(editor, (src) => window.playground.getSource() === src, craps);
  expect(linked, 'a link in the guide opens its example');

  // The cursor stands out from the editor's background, light and dark,
  // and so does a selection. (Firefox can't be switched from here, so it
  // checks the one it has.)
  for (const value of firefox ? [null] : ['light', 'dark']) {
    if (value) await editor.emulateMediaFeatures([{ name: 'prefers-color-scheme', value }]);
    const { scheme, ratio } = await editor.evaluate(() => {
      window.playground.view.focus();
      const luminance = (color) => {
        const [r, g, b] = color
          .match(/[\d.]+/g)
          .slice(0, 3)
          .map((c) => (c / 255 <= 0.03928 ? c / 255 / 12.92 : ((c / 255 + 0.055) / 1.055) ** 2.4));
        return 0.2126 * r + 0.7152 * g + 0.0722 * b;
      };
      const cursor = luminance(getComputedStyle(document.querySelector('.cm-cursor')).borderLeftColor);
      const background = luminance(getComputedStyle(window.playground.view.dom).backgroundColor);
      return {
        scheme: matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light',
        ratio: (Math.max(cursor, background) + 0.05) / (Math.min(cursor, background) + 0.05),
      };
    });
    expect(ratio >= 4.5, `the cursor shows against the ${scheme} background`, `contrast ${ratio.toFixed(1)}`);

    // A selection on the cursor's line isn't hidden: CodeMirror draws it
    // behind the text, so nothing over it may be opaque.
    const covered = await editor.evaluate(async () => {
      const { view } = window.playground;
      const line = view.state.doc.line(5);
      view.dispatch({ selection: { anchor: line.from + 4, head: line.from + 8 } });
      await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
      const at = view.coordsAtPos(line.from + 5);
      const stack = document.elementsFromPoint(at.left + 2, (at.top + at.bottom) / 2);
      const selection = stack.findIndex((e) => e.classList.contains('cm-selectionBackground'));
      if (selection < 0) return ['no selection drawn'];
      const alpha = (color) => (color.startsWith('rgba') ? parseFloat(color.split(',')[3]) : 1);
      return stack
        .slice(0, selection)
        .filter((e) => alpha(getComputedStyle(e).backgroundColor) > 0.5)
        .map((e) => e.className);
    });
    expect(covered.length === 0, `a selection on the cursor’s line shows (${scheme})`, covered.join('\n'));
  }

  // On a phone, the guide's long lines scroll by themselves, not the page.
  await editor.setViewport({ width: 390, height: 844 });
  await showTab('guide');
  const widths = await editor.evaluate(() => [document.documentElement.scrollWidth, window.innerWidth]);
  expect(widths[0] <= widths[1], 'on a phone, the page fits the screen', widths.join(' > '));
} finally {
  await browser.close();
  await server?.dispose();
}

console.log(failures ? `${failures} failed` : 'all passed');
process.exit(failures ? 1 : 0);
