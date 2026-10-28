import { chromium } from '/home/v9i/.hermes/hermes-agent/node_modules/playwright-core/index.mjs';
const [,, out, w, h, theme] = process.argv;
const browser = await chromium.launch({ executablePath: '/usr/bin/chromium' });
const page = await browser.newPage({ viewport: { width: +w, height: +h } });
if (theme) await page.addInitScript((t) => localStorage.setItem('kala-studio.theme', t), theme);
await page.goto('http://127.0.0.1:8791/');
await page.fill('input[type=password]', 'opus-look-token-0123456789abcdef');
await page.click('button:has-text("Connect")');
await page.waitForTimeout(2500);
for (const step of (process.env.STEPS || '').split(';').filter(Boolean)) {
  const [kind, arg] = step.split('=');
  if (kind === 'click') await page.click(arg);
  if (kind === 'wait') await page.waitForTimeout(+arg);
  if (kind === 'fill') { const [sel, val] = arg.split('|'); await page.fill(sel, val); }
  if (kind === 'press') await page.keyboard.press(arg);
}
await page.screenshot({ path: out });
await browser.close();
