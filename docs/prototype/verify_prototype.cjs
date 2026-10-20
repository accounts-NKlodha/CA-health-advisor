// Headless check for the simulated prototype (design review aid, not product code).
// Usage: NODE_PATH=<dir containing playwright> PLAYWRIGHT_BROWSERS_PATH=<browsers> node docs/prototype/verify_prototype.cjs
// Opens index.html from file://, clicks every flow in the reviewer index, clicks every visible action
// in each flow's first screen (restarting the flow each time), walks primary buttons, checks the
// keyboard path and pet drag, records console errors/warnings and any network request, and saves screenshots.
const path = require('path');
const { chromium } = require('playwright');

const file = 'file://' + path.resolve(__dirname, 'index.html');
const shots = path.resolve(__dirname, 'screens');
const SCOPE = '#bubble, #hyWin, #modalWrap.show #modal, #adminWin, #gallery.show, #capBox, #trayMenu';

(async () => {
  const browser = await chromium.launch(process.env.CHROMIUM_PATH ? { executablePath: process.env.CHROMIUM_PATH } : {});
  const page = await browser.newPage({ viewport: { width: 1366, height: 768 } });
  const problems = [], requests = [];
  page.on('console', m => { if (['error', 'warning'].includes(m.type())) problems.push(`${m.type()}: ${m.text()}`); });
  page.on('pageerror', e => problems.push('pageerror: ' + e.message));
  page.on('request', r => { if (!r.url().startsWith('file:') && !r.url().startsWith('data:')) requests.push(r.url()); });
  await page.goto(file);
  await page.waitForTimeout(300);

  const flows = await page.$$eval('#rev button.f:not([disabled])', bs => bs.map(b => b.dataset.v));
  const visibleActions = () => page.$$eval(`:is(${SCOPE}) [data-a]`, els => els
    .filter(e => e.offsetParent !== null && !e.disabled && e.getBoundingClientRect().width > 0)
    .map(e => e.dataset.a + '|' + (e.dataset.v || '')));
  const start = async f => { await page.click(`#rev button.f[data-v="${f}"]`); await page.waitForTimeout(250); };
  const clickAction = async key => {
    const [a, v] = key.split('|');
    const loc = page.locator(`:is(${SCOPE}) [data-a="${a}"][data-v="${v}"]`).filter({ visible: true }).first();
    if (await loc.count()) { await loc.click({ timeout: 2000 }).catch(e => problems.push(`click ${key}: ${e.message.split('\n')[0]}`)); await page.waitForTimeout(200); return true; }
    return false;
  };
  const walkPrimary = async n => {
    for (let i = 0; i < n; i++) {
      const loc = page.locator(`:is(${SCOPE}) .btn.primary[data-a]`).filter({ visible: true }).first();
      if (!(await loc.count())) break;
      await loc.click({ timeout: 2000 }).catch(() => {});
      await page.waitForTimeout(250);
    }
  };

  let clicks = 0;
  const perFlow = {};
  for (const f of flows) {
    await start(f);
    const first = await visibleActions();
    perFlow[f] = first.length;
    for (const key of first.slice(0, 45)) {
      await start(f);
      if (await clickAction(key)) clicks++;
      // one level deeper: click every action that appeared, then walk primaries
      const second = (await visibleActions()).filter(k => !first.includes(k)).slice(0, 12);
      for (const k2 of second) {
        // a modal opened by an earlier click covers the window behind it; close it unless k2 lives inside it
        const [a2, v2] = k2.split('|');
        const inModal = await page.locator(`#modalWrap.show #modal [data-a="${a2}"][data-v="${v2}"]`).count();
        if (!inModal && await page.$('#modalWrap.show')) { await page.keyboard.press('Escape'); await page.waitForTimeout(100); }
        if (await clickAction(k2)) clicks++;
      }
      await walkPrimary(4);
    }
    await start(f);
    await walkPrimary(12);
  }

  // keyboard path: Ctrl+Alt+H opens the panel with focus inside, Escape closes it
  await start('keyboard');
  await page.keyboard.press('Control+Alt+KeyH');
  await page.waitForTimeout(200);
  const focusedInBubble = await page.evaluate(() => document.querySelector('#bubble').contains(document.activeElement));
  await page.keyboard.press('Tab');
  await page.keyboard.press('Escape');
  await page.waitForTimeout(150);
  const bubbleClosed = await page.evaluate(() => !document.querySelector('#bubble').classList.contains('show'));
  if (!focusedInBubble) problems.push('keyboard: Ctrl+Alt+H did not move focus into the pet panel');
  if (!bubbleClosed) problems.push('keyboard: Escape did not close the pet panel');

  // pet drag to the left edge and back
  await page.click('#rev .sim [data-v="reset"]').catch(() => {});
  await start('desktop');
  await page.keyboard.press('Escape');
  const box = await page.locator('#pet').boundingBox();
  await page.mouse.move(box.x + 60, box.y + 60); await page.mouse.down();
  await page.mouse.move(500, 400, { steps: 8 }); await page.mouse.up();
  const left = await page.$eval('#pet', p => p.offsetLeft);
  if (left > 400) problems.push('drag: pet did not snap to the left edge');

  // screenshots
  const snap = async name => { await page.waitForTimeout(350); await page.screenshot({ path: path.join(shots, name) }); };
  await page.click('#rev .sim [data-v="reset"]');
  await start('reminder'); await snap('01-combined-reminder.png');
  await start('setup'); await clickAction('suNext|'); await page.check('input[data-v="lan"]'); await page.waitForTimeout(200);
  await clickAction('suCheck|'); await page.waitForTimeout(700); await snap('02-setup-pairing.png');
  await start('weekly'); await clickAction('openReview|'); await clickAction('wkVar|mixed'); await snap('03-weekly-review-mixed.png');
  await start('monthly'); await snap('04-monthly-progress.png');
  await start('sharing'); await clickAction('shToggle|hydration'); await snap('05-sharing-preview.png');
  await start('camera'); await clickAction('camStart|allowed'); await page.waitForTimeout(800); await snap('06-camera-calibration.png');

  await browser.close();
  console.log(JSON.stringify({ flows: flows.length, perFlowFirstScreenActions: perFlow, clicks, keyboard: { focusedInBubble, bubbleClosed }, petLeftAfterDrag: left, networkRequests: requests, problems }, null, 2));
  process.exit(problems.length || requests.length ? 1 : 0);
})();
