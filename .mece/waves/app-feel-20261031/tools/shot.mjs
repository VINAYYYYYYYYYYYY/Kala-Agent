// Usage: node tools/shot.mjs <port> <out.png> [width] [height] [theme]
// env STEPS="click=<selector>;wait=<ms>;fill=<selector>|<text>;press=<key>;role=<role>|<name regex>"
import { chromium } from '/home/v9i/.hermes/hermes-agent/node_modules/playwright-core/index.mjs';
const [,, port, out, w = '1440', h = '900', theme] = process.argv;
const browser = await chromium.launch({ executablePath: '/usr/bin/chromium' });
const page = await browser.newPage({ viewport: { width: +w, height: +h } });
page.on('console', (m) => { if (m.type() === 'error') console.log('console error:', m.text()); });
if (theme) await page.addInitScript((t) => localStorage.setItem('kala-studio.theme', t), theme);
await page.goto(`http://127.0.0.1:${port}/`);
await page.waitForTimeout(1500);
const pw = page.locator('input[type=password]');
if (await pw.count()) { await pw.first().fill('opus-look-token-0123456789abcdef'); await page.locator('button:has-text("Connect")').first().click(); }
await page.waitForTimeout(2500);
for (const step of (process.env.STEPS || '').split(';').filter(Boolean)) {
  const i = step.indexOf('='); const kind = step.slice(0, i); const arg = step.slice(i + 1);
  if (kind === 'click') await page.click(arg);
  if (kind === 'role') { const [role, name] = arg.split('|'); await page.getByRole(role, { name: new RegExp(name, 'i') }).first().click(); }
  if (kind === 'wait') await page.waitForTimeout(+arg);
  if (kind === 'fill') { const [sel, val] = arg.split('|'); await page.fill(sel, val); }
  if (kind === 'press') await page.keyboard.press(arg);
}
await page.screenshot({ path: out });
const n = await page.evaluate(() => [...document.querySelectorAll('button, [role=tab], [role=radio], [role=switch], select, input, textarea')].filter((e) => { const r = e.getBoundingClientRect(); return r.width > 0 && r.height > 0; }).length);
console.log(`${out}: ${n} visible controls`);
await browser.close();
