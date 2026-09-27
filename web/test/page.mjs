// The playground in real browsers, headless: every example prints what
// `probl run` prints, and stopping, limits, errors in the editor and share
// links work. Needs `npm run build`, `cargo build --release -p probl-cli`,
// and Chrome or Firefox where macOS puts them (or PROBL_CHROME and
// PROBL_FIREFOX).
//
//   node test/page.mjs            # Chrome
//   node test/page.mjs firefox    # Firefox

import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import * as esbuild from 'esbuild';
import puppeteer from 'puppeteer-core';

const root = fileURLToPath(new URL('../..', import.meta.url));
const firefox = process.argv.includes('firefox');
const browserPath = firefox
  ? process.env.PROBL_FIREFOX ?? '/Applications/Firefox.app/Contents/MacOS/firefox'
  : process.env.PROBL_CHROME ?? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';

const server = await esbuild.context({});
const { port } = await server.serve({ servedir: `${root}web/dist`, port: 0 });
const url = `http://127.0.0.1:${port}/`;
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

/** Run the editor's program, and wait for it to finish. */
async function run(page) {
  const before = await page.evaluate(() => window.playground.finished());
  await press(page, 'run');
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
  await new Promise((resolve) => setTimeout(resolve, 1500));
  const progress = await page.$eval('#status', (e) => e.textContent);
  expect(/Running… [\d,]+ of 100,000,000 runs/.test(progress), 'a long run shows its progress', progress);
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
} finally {
  await browser.close();
  await server.dispose();
}

console.log(failures ? `${failures} failed` : 'all passed');
process.exit(failures ? 1 : 0);
