// Deploy the playground to Cloudflare Pages: build it, create the Pages
// project the first time, and upload web/dist with wrangler.
//
//   npm run deploy                  # production
//   npm run deploy -- --preview     # a preview, named after the git branch
//
// It needs, from the environment or from web/.env (which git ignores):
//
//   CLOUDFLARE_API_TOKEN      a token with the permission
//                             Account · Cloudflare Pages · Edit
//   CLOUDFLARE_ACCOUNT_ID     the account's ID
//   CLOUDFLARE_PAGES_PROJECT  the project's name (probl-playground if unset)
//
// Building needs what `npm run build` needs: Rust, with the wasm32 target.

import { execFileSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const web = fileURLToPath(new URL('.', import.meta.url));
const root = fileURLToPath(new URL('..', import.meta.url));

function fail(message) {
  console.error(`\n${message}`);
  process.exit(1);
}

if (existsSync(join(web, '.env'))) process.loadEnvFile(join(web, '.env'));
const token = process.env.CLOUDFLARE_API_TOKEN;
const account = process.env.CLOUDFLARE_ACCOUNT_ID;
const project = process.env.CLOUDFLARE_PAGES_PROJECT || 'probl-playground';
const preview = process.argv.includes('--preview');
if (!token || !account) {
  fail(
    'Set CLOUDFLARE_API_TOKEN and CLOUDFLARE_ACCOUNT_ID, in the environment or in web/.env.\n' +
      'The token needs the permission Account · Cloudflare Pages · Edit: create one at\n' +
      'https://dash.cloudflare.com/profile/api-tokens. The account ID is on the account’s home page.',
  );
}

// ── What's being deployed ──
const git = (...args) => execFileSync('git', args, { cwd: root, encoding: 'utf8' }).trim();
const branch = git('rev-parse', '--abbrev-ref', 'HEAD');
const commit = git('rev-parse', 'HEAD');
const message = git('log', '-1', '--format=%s');
const dirty = git('status', '--porcelain') !== '';

// ── The project: its production branch and its address ──
const api = `https://api.cloudflare.com/client/v4/accounts/${account}/pages/projects`;
async function findProject() {
  const response = await fetch(`${api}/${encodeURIComponent(project)}`, {
    headers: { Authorization: `Bearer ${token}` },
  });
  const body = await response.json().catch(() => ({}));
  // Not found: the project, not the account (whose error is also a 404).
  if (response.status === 404 && (body.errors ?? []).some((e) => e.code === 8000007)) return null;
  if (!response.ok) {
    const errors = (body.errors ?? []).map((e) => `${e.code}: ${e.message}`).join('; ');
    const refused = response.status === 401 || response.status === 403 || /auth/i.test(errors);
    const hint = refused
      ? '\nCheck that the token is right, and has Account · Cloudflare Pages · Edit for this account.'
      : '';
    fail(`Cloudflare didn’t say whether the project ${project} exists (HTTP ${response.status}${errors ? `, ${errors}` : ''}).${hint}`);
  }
  return body.result;
}

const wranglerBin = join(dirname(createRequire(import.meta.url).resolve('wrangler/package.json')), 'bin', 'wrangler.js');
const wrangler = (...args) => execFileSync(process.execPath, [wranglerBin, ...args], { cwd: web, stdio: 'inherit' });

let found = await findProject();
if (!found) {
  console.log(`Creating the Pages project ${project}, which deploys main to production.`);
  wrangler('pages', 'project', 'create', project, '--production-branch', 'main');
  found = await findProject();
  if (!found) fail(`The project ${project} was created, but Cloudflare can’t find it.`);
}
const production = found.production_branch;
if (preview && branch === production) {
  fail(`On ${production}, the production branch, a deployment is the production one: leave out --preview.`);
}

// ── Build, then upload ──
if (dirty) console.warn('Warning: the working tree has changes that aren’t committed. They’re deployed too.\n');
execFileSync(process.execPath, ['build.mjs'], { cwd: web, stdio: 'inherit' });
wrangler(
  'pages',
  'deploy',
  'dist',
  '--project-name',
  project,
  '--branch',
  preview ? branch : production,
  '--commit-hash',
  commit,
  '--commit-message',
  message,
  `--commit-dirty=${dirty}`,
);

if (!preview) console.log(`\nThe playground is at https://${found.subdomain}`);
