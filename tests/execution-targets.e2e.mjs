// Build the UI and binary first. Run with Node and an installed Playwright Chromium.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { createServer } from 'node:net';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE ?? 'playwright');
const repo = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const binary = resolve(process.env.TASK_SERVER_BINARY ?? join(repo, 'target/debug/task-server'));
const root = await mkdtemp(join(tmpdir(), 'task-server-target-e2e-'));
const ledger = join(root, 'ledger');
const config = join(root, 'targets.yaml');
const evidence = process.env.E2E_EVIDENCE_DIR;
const labels = ['forge', 'field', `研究 / 試行 ${'長い実行先'.repeat(18)}`];
let server, base;
const checks = [];
const browser = await chromium.launch({ headless: true });

async function stop() {
  if (server && server.exitCode === null && server.signalCode === null) {
    const ended = once(server, 'exit');
    server.kill();
    await ended;
  }
}
async function start(configured = true) {
  await stop();
  const reservation = createServer();
  reservation.listen(0, '127.0.0.1');
  await once(reservation, 'listening');
  const port = reservation.address().port;
  await new Promise(resolve => reservation.close(resolve));
  base = `http://127.0.0.1:${port}`;
  server = spawn(binary, [], {
    cwd: root,
    env: { PORT: String(port), LOG_LEVEL: 'warn', APP_DATA_DIR: ledger,
      ...(configured ? { EXECUTION_TARGETS_FILE: config } : {}) },
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  let output = '';
  server.stdout.on('data', data => { output += data; });
  server.stderr.on('data', data => { output += data; });
  for (let attempt = 0; attempt < 200; attempt++) {
    assert.equal(server.exitCode, null, output);
    try { if ((await fetch(`${base}/healthz`)).ok) return; } catch { /* startup */ }
    await new Promise(resolve => setTimeout(resolve, 25));
  }
  throw new Error(`server did not start: ${output}`);
}
async function api(path, method = 'GET', body) {
  const response = await fetch(base + path, { method, headers: { 'content-type': 'application/json' },
    ...(body === undefined ? {} : { body: JSON.stringify(body) }) });
  assert.ok(response.ok, `${method} ${path}: ${response.status} ${await response.clone().text()}`);
  return response.status === 204 ? null : response.json();
}
async function screenshot(page, name) {
  if (evidence) await page.screenshot({ path: join(evidence, `${name}.png`), fullPage: true });
}
async function geometry(page) {
  const measured = await page.evaluate(() => {
    const row = document.querySelector('a.card');
    const style = row && getComputedStyle(row);
    return { width: innerWidth, scroll: document.documentElement.scrollWidth,
      rows: document.querySelectorAll('a.card').length,
      column: document.querySelector('.content').getBoundingClientRect().width,
      border: style?.borderWidth, radius: style?.borderRadius, padding: style?.padding,
      headerControls: document.querySelectorAll('header a, header button').length };
  });
  assert.ok(measured.scroll <= measured.width, JSON.stringify(measured));
  assert.equal(measured.headerControls, 4);
  assert.equal(measured.border, '1px');
  assert.equal(measured.radius, '8px');
  assert.equal(measured.padding, '10px');
  if (measured.width === 900) assert.equal(measured.column, 720);
  return measured;
}
try {
  if (evidence) await mkdir(evidence, { recursive: true });
  await mkdir(join(ledger, 'tasks'), { recursive: true });
  for (const [id, target, status] of [['legacy', undefined, 'draft'], ['removed', 'retired destination', 'draft'], ['historic', 'retired destination', 'done']]) {
    const fields = { id, title: `Reference ${id}`, status, product_id: 'example/project', kind: 'normal',
      ...(target === undefined ? {} : { execution_target: target }),
      created_at: '2026-09-01T00:00:00Z', updated_at: '2026-09-01T00:00:00Z', closed_at: status === 'done' ? '2026-09-02T00:00:00Z' : null };
    await writeFile(join(ledger, 'tasks', `${id}.md`), `---\n${Object.entries(fields).map(([key, value]) => `${key}: ${JSON.stringify(value)}`).join('\n')}\n---\nRetained source.\n`);
  }
  const originalLegacy = await readFile(join(ledger, 'tasks/legacy.md'), 'utf8');
  await writeFile(config, JSON.stringify({ labels, default: 'field' })); // JSON is valid YAML.
  await start();
  await api('/api/products/example/project', 'PUT', { repository: 'https://example.test/project', releases: false });
  assert.deepEqual(await api('/api/execution-targets'), { labels, default: 'field' });
  const context = await browser.newContext({ viewport: { width: 900, height: 900 }, colorScheme: 'dark' });
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', error => errors.push(String(error)));
  await page.goto(base);
  await page.getByRole('option', { name: labels[2], exact: true }).waitFor({ state: 'attached' });
  for (const label of labels) {
    await page.getByRole('button', { name: '新規タスク', exact: true }).click();
    const form = page.getByRole('dialog');
    await form.getByRole('option', { name: labels[2], exact: true }).waitFor({ state: 'attached' });
    assert.equal(await form.getByLabel('実行先', { exact: true }).inputValue(), 'field');
    await form.getByLabel('product', { exact: true }).fill('example/project');
    await form.getByLabel('title', { exact: true }).fill(`Created ${label}`);
    await form.getByLabel('body', { exact: true }).fill('## Instructions\nPreserve source text.');
    await form.getByLabel('実行先', { exact: true }).selectOption(label);
    await form.getByRole('button', { name: '保存', exact: true }).click();
    await form.waitFor({ state: 'hidden' });
    const created = (await api('/api/tasks')).find(task => task.title === `Created ${label}`);
    assert.equal(created.execution_target, label);
    await page.getByLabel('実行先で絞り込み').selectOption(label);
    await page.locator(`a[href="/tasks/${created.id}"]`).waitFor();
    for (const caption of await page.locator('[data-field="execution-target"]').allTextContents()) assert.equal(caption.trim(), label);
    await page.getByLabel('実行先で絞り込み').selectOption({ label: 'すべて' });
  }
  checks.push('external default and all three names reach real create/filter APIs');
  const created = (await api('/api/tasks')).find(task => task.title === 'Created forge');
  await page.locator(`a[href="/tasks/${created.id}"]`).click();
  await page.getByRole('button', { name: '編集', exact: true }).click();
  await page.getByRole('dialog').getByRole('option', { name: labels[2], exact: true }).waitFor({ state: 'attached' });
  await page.getByRole('dialog').getByLabel('実行先', { exact: true }).selectOption(labels[2]);
  await page.getByRole('dialog').getByRole('button', { name: '保存', exact: true }).click();
  await page.getByRole('dialog').waitFor({ state: 'hidden' });
  assert.equal((await api(`/api/tasks/${created.id}`)).execution_target, labels[2]);
  await page.goto(`${base}/tasks/removed`);
  await page.getByRole('button', { name: '編集', exact: true }).click();
  await page.getByRole('dialog').getByRole('option', { name: /retired destination.*現在の設定にありません/ }).waitFor({ state: 'attached' });
  await page.getByRole('dialog').getByLabel('title', { exact: true }).fill('Unrelated edit');
  const patchRequest = page.waitForRequest(request => request.method() === 'PATCH');
  await page.getByRole('dialog').getByRole('button', { name: '保存', exact: true }).click();
  assert.ok(!Object.hasOwn((await patchRequest).postDataJSON(), 'execution_target'));
  await page.getByRole('dialog').waitFor({ state: 'hidden' });
  assert.equal((await api('/api/tasks/removed')).execution_target, 'retired destination');
  await page.goto(`${base}/closed`);
  await page.getByText('retired destination（現在の設定にありません）', { exact: true }).waitFor();
  assert.equal(await readFile(join(ledger, 'tasks/legacy.md'), 'utf8'), originalLegacy);
  checks.push('edits and closed history preserve removed references; reads do not rewrite legacy files');
  if (evidence) await writeFile(join(evidence, 'visual-tasks.json'), JSON.stringify(await api('/api/tasks'), null, 2));

  for (const width of [900, 375, 320]) for (const theme of ['dark', 'light']) {
    await page.setViewportSize({ width, height: 900 });
    await page.emulateMedia({ colorScheme: theme });
    await page.goto(base);
    await page.locator('a.card').first().waitFor();
    const measured = await geometry(page);
    await screenshot(page, `after-${width}-${theme}-home`);
    await page.getByRole('button', { name: '新規タスク', exact: true }).focus();
    await page.keyboard.press('Enter');
    await page.getByRole('dialog').getByRole('option', { name: labels[2], exact: true }).waitFor({ state: 'attached' });
    await page.getByRole('dialog').getByLabel('product', { exact: true }).fill('example/project');
    await page.getByRole('dialog').getByLabel('title', { exact: true }).fill('Draft to preserve');
    await page.getByRole('dialog').getByLabel('実行先', { exact: true }).focus();
    await page.keyboard.press('ArrowDown');
    assert.equal(await page.getByRole('dialog').getByLabel('実行先', { exact: true }).inputValue(), labels[2]);
    assert.equal(await page.getByRole('dialog').getByLabel('実行先', { exact: true }).evaluate(el => getComputedStyle(el).outlineWidth), '2px');
    await screenshot(page, `after-${width}-${theme}-form`);
    assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
    await page.keyboard.press('Escape');
    await page.getByRole('dialog').waitFor({ state: 'hidden' });
    assert.equal(await page.evaluate(() => document.activeElement?.textContent?.trim()), '新規タスク');
    checks.push({ width, theme, measured });
  }
  await page.route('**/api/execution-targets', route => route.fulfill({ status: 503, json: { error: 'offline' } }));
  await page.goto(base);
  await page.getByText('実行先の設定を読み込めませんでした', { exact: true }).waitFor();
  assert.ok(await page.locator('a.card').count() > 0);
  await page.getByRole('button', { name: '新規タスク', exact: true }).click();
  await page.getByRole('dialog').getByText('実行先の設定を読み込めませんでした').waitFor();
  await page.getByRole('dialog').getByLabel('product', { exact: true }).fill('example/project');
  await page.getByRole('dialog').getByLabel('title', { exact: true }).fill('Retry draft');
  assert.equal(await page.getByRole('dialog').getByRole('button', { name: '保存', exact: true }).getAttribute('aria-disabled'), 'true');
  await screenshot(page, 'configuration-failure');
  await page.unroute('**/api/execution-targets');
  await page.getByRole('dialog').getByRole('button', { name: '実行先を再読み込み' }).click();
  await page.getByRole('dialog').getByRole('option', { name: 'field', exact: true }).waitFor({ state: 'attached' });
  assert.equal(await page.getByRole('dialog').getByLabel('title', { exact: true }).inputValue(), 'Retry draft');
  await page.keyboard.press('Escape');
  checks.push('configuration failure keeps list and form data; retry restores choices');

  await writeFile(config, 'labels: [island, field]\n');
  await start();
  assert.deepEqual(await api('/api/execution-targets'), { labels: ['island', 'field'], default: null });
  await page.goto(base);
  await page.getByRole('option', { name: 'island', exact: true }).waitFor({ state: 'attached' });
  await page.getByLabel('実行先で絞り込み').selectOption({ label: '未設定' });
  await page.locator('a[href="/tasks/legacy"]').waitFor();
  assert.equal(await page.locator('a.card').count(), 1);
  await page.getByRole('button', { name: '新規タスク', exact: true }).click();
  await page.getByRole('dialog').getByRole('option', { name: 'island', exact: true }).waitFor({ state: 'attached' });
  assert.equal(await page.getByRole('dialog').getByLabel('実行先', { exact: true }).inputValue(), '');
  await page.getByRole('dialog').getByLabel('product', { exact: true }).fill('example/project');
  await page.getByRole('dialog').getByLabel('title', { exact: true }).fill('After restart');
  assert.equal(await page.getByRole('dialog').getByRole('button', { name: '保存', exact: true }).getAttribute('aria-disabled'), 'true');
  await page.getByRole('dialog').getByLabel('実行先', { exact: true }).selectOption('island');
  await page.getByRole('dialog').getByRole('button', { name: '保存', exact: true }).click();
  await page.getByRole('dialog').waitFor({ state: 'hidden' });
  assert.equal((await api('/api/tasks')).find(task => task.title === 'After restart').execution_target, 'island');
  checks.push('same binary reloads replaced choices and absent default; unset legacy remains filterable');

  await start(false);
  assert.deepEqual(await api('/api/execution-targets'), { labels: [], default: null });
  await page.goto(base);
  await page.getByText('実行先が設定されていません', { exact: true }).waitFor();
  await page.getByRole('button', { name: '新規タスク', exact: true }).click();
  await page.getByRole('dialog').getByText('実行先が設定されていません').waitFor();
  assert.equal(await page.getByRole('dialog').getByLabel('実行先', { exact: true }).isDisabled(), true);
  await screenshot(page, 'configuration-absent');
  await page.keyboard.press('Escape');
  await page.goto(`${base}/tasks/legacy`);
  await page.getByRole('button', { name: '編集', exact: true }).click();
  await page.getByRole('dialog').getByText('実行先が設定されていません').waitFor();
  await page.getByRole('dialog').getByLabel('title', { exact: true }).fill('Edited unset record');
  await page.getByRole('dialog').getByRole('button', { name: '保存', exact: true }).click();
  await page.getByRole('dialog').waitFor({ state: 'hidden' });
  assert.equal((await api('/api/tasks/legacy')).execution_target, null);
  checks.push('absent configuration disables new selection but permits unrelated legacy edits');
  assert.deepEqual(errors, []);
  await context.close();
  if (evidence) await writeFile(join(evidence, 'browser-results.json'), JSON.stringify(checks, null, 2));
  console.log(JSON.stringify({ passed: true, checks }));
} finally {
  await stop();
  await browser.close();
  await rm(root, { recursive: true, force: true });
}
