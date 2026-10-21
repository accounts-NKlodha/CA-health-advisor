// Headless check for the simulated prototype (design review aid, not product code).
// Usage: NODE_PATH=<dir containing playwright> PLAYWRIGHT_BROWSERS_PATH=<browsers> node docs/prototype/verify_prototype.cjs
// Opens index.html from file://, clicks every flow in the reviewer index, clicks every visible action
// in each flow's first screen (restarting the flow each time), walks primary buttons, checks the
// keyboard path and pet drag, records console errors/warnings and any network request, saves screenshots,
// and runs targeted STATE assertions (revision 2: persistence and undo, range deletion, focus, contract wording, water copy, proposed defaults).
const path = require('path');
const fs = require('fs');
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
  for (const f of process.env.ASSERT_ONLY ? [] : flows) { // ASSERT_ONLY=1 skips the long click walk (quick iteration only; the committed run is full)
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

  const snapReady = async (name, setup) => { await setup(); await page.waitForTimeout(350); await page.screenshot({ path: path.join(shots, name) }); };

  // ---------------------------------------------------------------------------------------------
  // Targeted state assertions. They read the in-memory state object S (a global in index.html) or DOM values.
  // ---------------------------------------------------------------------------------------------
  const results = []; let group = '';
  const G = g => { group = g; };
  const check = (name, cond, detail = '') => { results.push({ group, name, ok: !!cond }); if (!cond) problems.push(`ASSERT FAILED [${group}] ${name} ${detail}`); };
  const ev = (fn, arg) => page.evaluate(fn, arg);
  const reset = async () => { await ev(() => F.reset()); await page.waitForTimeout(120); };
  const bubbleText = () => ev(() => document.querySelector('#bubble').innerText);
  const modalText = () => ev(() => document.querySelector('#modal').innerText);
  const click = async sel => { await page.locator(sel).filter({ visible: true }).first().click({ timeout: 3000 }); await page.waitForTimeout(150); };
  const src = fs.readFileSync(path.resolve(__dirname, 'index.html'), 'utf8');

  // ---- 1. Persistence and undo
  G('1 persistence+undo');
  await reset(); await start('recovery');
  await click('#bubble [data-a="recYes"]'); await click('#bubble [data-a="recPlan"][data-v="Wed 22 Oct"]');
  check('recovery window opens with a draft in S', await ev(() => !!S.recDraft && S.recovery === null));
  await click('#hyBody [data-a="recSw"][data-v="lunch"]');
  await click('#hyBody [data-a="recSw"][data-v="calm"]');
  check('lunch switch aria-checked false and S.recDraft.lunch false', await ev(() => document.querySelector('[data-a="recSw"][data-v="lunch"]').getAttribute('aria-checked') === 'false' && S.recDraft.lunch === false));
  await page.fill('#recWalk', '16:30'); await page.check('#recRepeat'); await page.fill('#recShut', '18:00');
  check('time fields and repeat bind to S.recDraft', await ev(() => S.recDraft.walk === '16:30' && S.recDraft.repeat === true && S.recDraft.shutdown === '18:00'));
  check('nothing is saved before Save plan (S.recovery still null)', await ev(() => S.recovery === null));
  await click('#hyBody [data-a="recSave"]');
  check('recSave stores the plan in S.recovery', await ev(() => S.recovery && S.recovery.lunch === false && S.recovery.calm === false && S.recovery.breaks === true && S.recovery.walk === '16:30' && S.recovery.repeat === true && S.recovery.shutdown === '18:00' && S.recDraft === null));
  await click('#capBox [data-a="recDay"]');
  const dayText = await bubbleText();
  check('recovery morning lists saved items only (no lunch, repeat shown)', !/Lunch away/.test(dayText) && /repeated every 90 min/.test(dayText) && /16:30/.test(dayText));
  await click('#bubble [data-a="recWin"]');
  check('reopened window shows saved values (DOM)', await ev(() => document.querySelector('#recWalk').value === '16:30' && document.querySelector('#recRepeat').checked && document.querySelector('#recShut').value === '18:00'
    && document.querySelector('[data-a="recSw"][data-v="lunch"]').getAttribute('aria-checked') === 'false' && document.querySelector('[data-a="recSw"][data-v="calm"]').getAttribute('aria-checked') === 'false' && document.querySelector('[data-a="recSw"][data-v="breaks"]').getAttribute('aria-checked') === 'true'));
  await click('#hyBody [data-a="recSw"][data-v="breaks"]'); await click('#hyBody [data-a="recDiscard"]');
  check('Discard changes leaves the saved plan unchanged', await ev(() => S.recovery.breaks === true && S.recDraft === null));
  // settings time fields
  await ev(() => settings('goals')); await page.fill('[data-in="hoursStart"]', '08:45'); await page.selectOption('[data-in="reviewAt"]', 'Fri 18:00');
  await ev(() => settings('reminders')); await ev(() => settings('goals'));
  check('work-hours start and review time persist after reopening the tab', await ev(() => S.hours[0] === '08:45' && S.reviewAt === 'Fri 18:00' && document.querySelector('[data-in="hoursStart"]').value === '08:45' && document.querySelector('[data-in="reviewAt"]').value === 'Fri 18:00'));
  // suggestions: apply and undo for three kinds
  await reset(); await start('weekly'); await click('#bubble [data-a="openReview"]'); await click('#hyBody [data-a="wkVar"][data-v="mixed"]');
  const before = await ev(() => ({ a: S.afternoonAt, sv: S.servings.slice(), mv: S.mvDefault }));
  check('suggestion baseline values', before.a === '15:00' && before.sv.join() === '150,250,500' && before.mv === 'Ask each time');
  for (const k of ['afternoon', 'serving', 'seated']) await click(`#sug-${k} [data-a="sug"]`);
  check('Apply changes S (afternoon 16:00, 750 ml button, seated default)', await ev(() => S.afternoonAt === '16:00' && S.servings.join() === '150,250,500,750' && S.mvDefault === 'Seated'));
  check('applied rows show Applied and Undo', await ev(() => ['afternoon', 'serving', 'seated'].every(k => /Applied/.test(document.querySelector('#sug-' + k).innerText) && document.querySelector(`#sug-${k} [data-a="sugUndo"]`))));
  await ev(() => A.openMonth()); await ev(() => A.openReview());
  check('reopening the review still shows Applied (state read from S)', await ev(() => /Applied/.test(document.querySelector('#sug-afternoon').innerText)));
  await ev(() => settings('reminders'));
  check('Reminders tab shows the new afternoon time', await ev(() => /16:00/.test(document.querySelector('#afternoonLine').innerText)));
  await ev(() => A.openReview());
  for (const k of ['afternoon', 'serving', 'seated']) await click(`#sug-${k} [data-a="sugUndo"]`);
  check('Undo restores the prior values exactly', await ev(() => S.afternoonAt === '15:00' && S.servings.join() === '150,250,500' && S.mvDefault === 'Ask each time' && !S.sugApplied.afternoon));
  check('no data-a=noop left on an Undo control', !/data-a="noop">Undo/.test(src) && !/btn\('Undo', 'noop'/.test(src));

  // ---- 2. Deletion by date range
  G('2 range deletion');
  await reset(); await start('sharing');
  const delBtn = k => `#hyBody [data-a="shDelete"][data-v="${k}"]`;
  const openDelete = async k => { await click(delBtn(k)); };
  await openDelete('hydration');
  check('date inputs are hidden until "A date range" is chosen', await ev(() => document.querySelector('#delRange').hidden === true));
  await click('#modal input[data-v="range"]');
  check('From/To date inputs appear for a range', await ev(() => !document.querySelector('#delRange').hidden && document.querySelector('#delFrom').type === 'date' && document.querySelector('#delTo').type === 'date'));
  await page.fill('#delFrom', '2026-10-16'); await page.fill('#delTo', '2026-10-12');
  await click('#modal [data-a="shDelAsk"]');
  check('From after To is rejected with a message and no confirmation', await ev(() => /must not be after/.test(document.querySelector('#delErr').innerText) && !document.querySelector('#delConfirmRange')));
  await page.fill('#delFrom', '2026-10-12'); await page.fill('#delTo', '2026-10-16');
  await click('#modal [data-a="shDelAsk"]');
  const conf = await modalText();
  check('confirmation names the chosen range, not all dates', /12 Oct 2026 to 16 Oct 2026/.test(conf) && !/all dates/i.test(conf.split('Cancel')[0].replace(/never be uploaded again/, '')) && /can never be uploaded again/.test(conf));
  check('range without today does not mention today', !/includes today/i.test(conf));
  await click('#modal [data-a="shDelGo"]'); await page.waitForTimeout(1100);
  const rc = await ev(() => document.querySelector('#delReceipt')?.innerText || '');
  check('receipt shows the chosen dates (12 Oct 2026 to 16 Oct 2026)', /12 Oct 2026 to 16 Oct 2026/.test(rc));
  check('receipt for a range does not say "all dates"', !/all dates/i.test(rc));
  check('tombstone recorded in S for the range', await ev(() => S.tombstones.length === 1 && S.tombstones[0].mode === 'range' && S.tombstones[0].from === '2026-10-12' && S.tombstones[0].to === '2026-10-16' && S.tombstones[0].today === false));
  await click('#modal [data-a="closeModal"]');
  check('Sharing tab lists the deleted range as never uploaded again', await ev(() => /12 Oct 2026 to 16 Oct 2026/.test(document.querySelector('#tombList').innerText)));
  // enabling after a range that excludes today still uploads today
  await click('#hyBody [data-a="shToggle"][data-v="hydration"]'); await click('#modal [data-a="shConfirm"]');
  check('range excluding today does not block today (1 summary waiting)', await ev(() => S.sharing.hydration === true && S.pending.hydration === 1));
  // range that includes today
  await openDelete('hydration'); await click('#modal input[data-v="range"]');
  await page.fill('#delFrom', '2026-10-18'); await page.fill('#delTo', '2026-10-20');
  await click('#modal [data-a="shDelAsk"]');
  check('range including today warns that today is not uploaded again', /includes today/i.test(await modalText()));
  await click('#modal [data-a="shDelGo"]'); await page.waitForTimeout(1100);
  const rc2 = await ev(() => document.querySelector('#delReceipt')?.innerText || '');
  check('receipt shows 18 Oct 2026 to 20 Oct 2026 and the today rule', /18 Oct 2026 to 20 Oct 2026/.test(rc2) && /Today is included/.test(rc2) && !/all dates/i.test(rc2));
  check('today deletion clears today\'s waiting summary', await ev(() => S.pending.hydration === 0));
  await click('#modal [data-a="closeModal"]');
  await click('#hyBody [data-a="shToggle"][data-v="hydration"]'); await click('#modal [data-a="closeModal"]'); // switch off, close "stopped"
  await click('#hyBody [data-a="shToggle"][data-v="hydration"]');
  check('M1 preview says today was deleted', /you deleted today/.test(await modalText()));
  await click('#modal [data-a="shConfirm"]');
  check('re-enabling after deleting today does NOT re-upload today (0 waiting)', await ev(() => S.sharing.hydration === true && S.pending.hydration === 0));
  // all dates
  await openDelete('outcomes');
  await click('#modal [data-a="shDelAsk"]');
  check('all-dates confirmation says all dates', /all dates/.test(await modalText()));
  await click('#modal [data-a="shDelGo"]'); await page.waitForTimeout(1100);
  const rc3 = await ev(() => document.querySelector('#delReceipt')?.innerText || '');
  check('all-dates receipt says "all dates" only for all dates', /all dates/.test(rc3) && !/ to \d+ \w+ 2026/.test(rc3));
  await click('#modal [data-a="closeModal"]');
  check('all-dates deletion includes today (tombstone today=true)', await ev(() => S.tombstones.some(t => t.k === 'outcomes' && t.mode === 'all' && t.today === true)));
  // offline queued variant
  await ev(() => A.setServer('offline')); await ev(() => settings('sharing')); await openDelete('phone');
  await click('#modal input[data-v="range"]'); await page.fill('#delFrom', '2026-10-01'); await page.fill('#delTo', '2026-10-05');
  await click('#modal [data-a="shDelAsk"]'); await click('#modal [data-a="shDelGo"]');
  check('offline deletion is queued and names the chosen range', /queued/i.test(await modalText()) && /1 Oct 2026 to 5 Oct 2026/.test(await modalText()));
  await click('#modal [data-a="closeModal"]');

  // ---- 3. Accessibility: modal focus and keyboard panel
  G('3 focus');
  await reset(); await start('sharing');
  const inv = '#hyBody [data-a="shDelete"][data-v="hydration"]';
  const isInv = () => ev(() => { const a = document.activeElement; return a && a.dataset.a === 'shDelete' && a.dataset.v === 'hydration' && a.closest('#hyBody') !== null; });
  const inModal = () => ev(() => document.querySelector('#modal').contains(document.activeElement));
  await page.focus(inv); await page.keyboard.press('Enter'); await page.waitForTimeout(150);
  check('modal opens and focus moves inside', await inModal());
  const nf = await ev(() => modalFocusables().length);
  check('modal has several focusable controls', nf >= 3, 'count ' + nf);
  let allIn = true; for (let i = 0; i < nf * 2 + 3; i++){ await page.keyboard.press('Tab'); if (!(await inModal())) allIn = false; }
  check(`Tab x${nf * 2 + 3} stays inside the modal (wraps)`, allIn);
  allIn = true; for (let i = 0; i < nf * 2 + 3; i++){ await page.keyboard.press('Shift+Tab'); if (!(await inModal())) allIn = false; }
  check(`Shift+Tab x${nf * 2 + 3} stays inside the modal (wraps)`, allIn);
  await ev(() => modalFocusables()[0].focus()); await page.keyboard.press('Shift+Tab');
  check('Shift+Tab on the first control goes to the last', await ev(() => { const f = modalFocusables(); return document.activeElement === f[f.length - 1]; }));
  await page.keyboard.press('Tab');
  check('Tab on the last control goes to the first', await ev(() => document.activeElement === modalFocusables()[0]));
  await page.keyboard.press('Escape'); await page.waitForTimeout(100);
  check('Escape closes the modal', await ev(() => !document.querySelector('#modalWrap').classList.contains('show')));
  check('Escape restores focus to the invoking control', await isInv());
  await click(inv); check('modal reopened by mouse click has focus inside', await inModal());
  await click('#modal [data-a="closeModal"]');
  check('Cancel closes the modal and restores focus to the invoker', await isInv() && await ev(() => !document.querySelector('#modalWrap').classList.contains('show')));
  await click('#hyBody [data-a="shToggle"][data-v="phone"]'); await page.waitForTimeout(100);
  const swInModal = await inModal(); await page.keyboard.press('Escape'); await page.waitForTimeout(100);
  check('focus returns to the switch that opened a preview modal (re-found after re-render)', swInModal && await ev(() => { const a = document.activeElement; return a.dataset.a === 'shToggle' && a.dataset.v === 'phone'; }));
  // keyboard panel
  await start('keyboard'); await page.keyboard.press('Control+Alt+KeyH'); await page.waitForTimeout(200);
  check('Ctrl+Alt+H puts focus inside the pet panel', await ev(() => document.querySelector('#bubble').contains(document.activeElement)));
  await page.keyboard.press('Tab'); await page.keyboard.press('Escape'); await page.waitForTimeout(120);
  check('panel Escape closes the panel and focuses the simulated work surface', await ev(() => document.activeElement && document.activeElement.id === 'workSurface' && !document.querySelector('#bubble').classList.contains('show')));
  check('real Windows focus limits stay labelled (caption and wireframes note A8)', /build acceptance test/.test(await ev(() => document.querySelector('#capBox').innerText)));

  // ---- 4. Reconcile with architecture contracts
  G('4 contracts');
  check('no timing-bitmap or time-slot wording in the prototype source', !/bitmap|\bslots?\b/i.test(src));
  await reset(); await start('sharing'); await click('#hyBody [data-a="shPreview"][data-v="sessions"]');
  const pv = await modalText();
  check('work-session preview has no within-day timing and no per-hour fields', /no within-day timing/.test(pv) && /longest block/.test(pv) && !/hourly|per hour|bitmap/i.test(pv));
  check('preview states summaries are per device and durations never summed', /per device/.test(pv) && /never added together across PCs/.test(pv));
  check('preview carries the Draft label', await ev(() => !!document.querySelector('#modal .draft')));
  await click('#modal [data-a="closeModal"]');
  await start('setup'); await click('#hyBody [data-a="suNext"]'); await page.check('input[data-v="lan"]'); await page.waitForTimeout(150);
  const slipV = await page.inputValue('#slip'); const parts = slipV.split('-');
  check('sample slip: hy1 prefix, 52-char fingerprint, 16-char secret', parts[0] === 'hy1' && parts[3].length === 52 && parts[4].length === 16, slipV);
  await click('#hyBody [data-a="suCheck"]'); await page.waitForTimeout(700);
  const okTxt = await ev(() => document.querySelector('#slipResult').innerText);
  check('full certificate fingerprint check is described as authoritative', /full certificate fingerprint matched/i.test(okTxt) && /authoritative/.test(okTxt));
  check('six words are labelled auxiliary and not the verification', /Auxiliary/.test(okTxt) && /not the verification/.test(okTxt) && !/They match - pair/.test(okTxt));
  await ev(() => A.slipOutcome('mismatch')); await ev(() => A.suCheck()); await page.waitForTimeout(700);
  check('fingerprint mismatch aborts before the secret is sent', /before sending the slip's secret/.test(await ev(() => document.querySelector('#slipResult').innerText)));
  await start('admin'); await click('#adminWin [data-a="adSlip"]');
  check('admin slip card calls the words auxiliary', /not the verification/.test(await ev(() => document.querySelector('#adminBody').innerText)));
  check('restore rule is shown with the Draft label', await ev(() => /outside the restored backup set/.test(document.querySelector('#adminBody').innerText) && !!document.querySelector('#adminBody #restoreHead .draft')));
  await click('#adminWin [data-a="adRestore"][data-v="closed"]');
  check('restore without proof fails closed: devices revoked, all categories off, unpaired', await ev(() => S.restore === 'closed' && S.server === 'none' && Object.values(S.sharing).every(v => v === false) && /Failed closed/.test(document.querySelector('#restoreResult').innerText) && /Revoked: must re-pair/.test(document.querySelector('#adminBody').innerText)));
  await ev(() => settings('sharing'));
  check('Sharing tab explains the fail-closed outcome honestly', await ev(() => /discarded/.test(document.querySelector('#restoreNote').innerText) && /Pair again/.test(document.querySelector('#restoreNote').innerText)));
  await ev(() => A.closeAdmin()); await snapReady('08-admin-restore-fail-closed.png', async () => { await ev(() => { closeWin(); adminWin(); }); });
  await click('#adminWin [data-a="adRestore"][data-v="proved"]');
  check('release happens only when a surviving control log proves completeness', await ev(() => S.restore === 'proved' && /History released/.test(document.querySelector('#restoreResult').innerText) && /control log/.test(document.querySelector('#restoreResult').innerText)));
  check('Draft pending contract label on sharing, backup, admin and pairing screens', await (async () => { const out = []; for (const t of ['sharing', 'backup']) { await ev(t2 => settings(t2), t); out.push(await ev(() => !!document.querySelector('#hyBody .draft'))); } out.push(await ev(() => !!document.querySelector('#adminBody .draft'))); return out.every(Boolean); })());

  // ---- 5. Water copy and pace
  G('5 water + pace');
  check('old "Nothing more needed" copy is gone from the source', !/nothing more needed/i.test(src));
  await reset();
  await ev(() => A.logW(2000)); await page.waitForTimeout(100);
  const gt = await bubbleText();
  check('quick-log confirmation uses the new goal-reached copy', /Your logging goal is reached; hydration reminders are paused/.test(gt));
  check('manual logging stays available (Undo and Details present)', await ev(() => !!document.querySelector('#bubble [data-a="wUndo"]') && !!document.querySelector('#bubble [data-a="waterPanel"]')));
  await ev(() => A.waterPanel()); await page.waitForTimeout(100);
  check('water panel shows new copy and still has serving buttons', /Your logging goal is reached; hydration reminders are paused/.test(await bubbleText()) && await ev(() => document.querySelectorAll('#bubble [data-a="logW"]').length >= 3));
  await ev(() => A.logW(250, true)); 
  check('logging above the goal still works (manual logging available)', await ev(() => waterTotal() === 2000 + 500 + 250));
  await ev(() => F.reminder()); await page.waitForTimeout(150);
  const r1 = await bubbleText();
  check('pace model: 11:15 with 250 ml logged is behind (expected 390 ml)', await ev(() => paceExpected() === 390 && behindPace() === true) && /140 ml behind your own pace/.test(r1) && /about 390 ml expected/.test(r1));
  check('reminder labels the sample scenario', /Sample scenario/.test(await ev(() => document.querySelector('#capBox').innerText)));
  await ev(() => { S.water = [{ id: 1, t: '09:50', ml: 500 }]; showReminder(); }); await page.waitForTimeout(100);
  check('ahead of pace (500 ml at 11:15): no water reason is listed', !/behind your own pace/.test(await bubbleText()) && await ev(() => behindPace() === false));
  await ev(() => { S.water = [{ id: 1, t: '09:50', ml: 2000 }]; showReminder(); }); await page.waitForTimeout(100);
  check('goal reached: water not listed and reminders say so', !/Water:/.test(await bubbleText()) && /logging goal is reached/.test(await bubbleText()));
  await reset(); await start('pomodoro'); await ev(() => A.pomoStart('25/5')); await ev(() => A.pomoBreak()); await page.waitForTimeout(100);
  const pb1 = await bubbleText();
  check('pomoBreak behind pace: 500 ml logged at 15:00 vs about 1220 ml expected', /720 ml behind your own pace/.test(pb1) && /about 1220 ml expected/.test(pb1));
  await ev(() => { S.water = [{ id: 1, t: '10:00', ml: 1400 }]; A.pomoBreak(); }); await page.waitForTimeout(100);
  check('below daily total (1400 of 2000) but ahead of pace is NOT called behind', await ev(() => waterTotal() < S.waterGoal && behindPace() === false) && !/behind your own pace|Water:/.test(await bubbleText()));
  check('break card labels the sample scenario in the caption', /Sample scenario/.test(await ev(() => document.querySelector('#capBox').innerText)));

  // ---- Proposed defaults (all labelled, none are owner answers)
  G('6 proposed defaults');
  await reset(); await ev(() => settings('reminders'));
  check('presentation default carries the Proposed default label', await ev(() => /Proposed default, owner to confirm/.test(document.querySelector('#hyBody').innerText) && /hidden completely/.test(document.querySelector('#hyBody').innerText) && /tray icon stays/.test(document.querySelector('#hyBody').innerText)));
  await start('focus'); await ev(() => A.simPresent());
  check('presentation: pet hidden, no bubble, tray stays', await ev(() => document.querySelector('#pet').style.display === 'none' && !document.querySelector('#bubble').classList.contains('show') && document.querySelector('#trayHY').offsetParent !== null));
  await ev(() => say('<p>test</p>'));
  check('presentation: bubbles are suppressed entirely', await ev(() => !document.querySelector('#bubble').classList.contains('show')));
  await ev(() => A.presentEnd()); await page.waitForTimeout(100);
  check('presentation end: pet returns with no replay burst (no bubble)', await ev(() => document.querySelector('#pet').style.display !== 'none' && !document.querySelector('#bubble').classList.contains('show')));
  await start('recovery');
  const ci = await bubbleText();
  check('9-hour check-in: labelled proposed, an estimate, not evidence of stress', /about 9 hours/.test(ci) && /estimate/.test(ci) && /not a sign of stress/.test(ci) && await ev(() => !!document.querySelector('#bubble .proposed')));
  await ev(() => settings('reminders')); await page.selectOption('[data-in="longDayHrs"]', '10');
  check('check-in hours are configurable (S.longDayHrs)', await ev(() => S.longDayHrs === 10));
  await ev(() => A.recAsk());
  await ev(() => { A.longDay(); recCheckIn(true); });
  check('check-in can be switched off (S.longDay false, automatic prompt not shown)', await ev(() => S.longDay === false) && /check-in is off/.test(await bubbleText()));
  await reset(); await start('camera'); await ev(() => { S.camera = 'on'; A.camCue(); });
  const cue = await bubbleText();
  check('posture cue is neutral by default (exact wording, no direction)', /Your position has changed from your calibration\. Would you like a short movement break\?/.test(cue) && !/leaning|forward|slouch/i.test(cue));
  check('posture cue labelled proposed and mentions the detector quality gate', await ev(() => !!document.querySelector('#bubble .proposed')) && /quality gate/.test(cue));
  await ev(() => settings('pet'));
  const pt = await ev(() => document.querySelector('#hyBody').innerText);
  check('pet set: visual-family approval is distinct from the ten finished pets (4 drawn, 6 art pending)', /Visual-family approval/.test(pt) && /4 are drawn/.test(pt) && /6 are art pending/.test(pt) && /Approving the family does not approve/.test(pt));
  await ev(() => settings('movement'));
  check('movement content is marked provisional pending content review', await ev(() => /Provisional/.test(document.querySelector('#hyBody').innerText) && /pending the separate content review/.test(document.querySelector('#hyBody').innerText)));
  check('simulation banner is intact', await ev(() => document.querySelector('#banner').innerText === 'Simulated prototype for design review. Not the product. No data is saved or sent.'));

  const passed = results.filter(r => r.ok).length, failed = results.length - passed;
  const byGroup = {}; results.forEach(r => { byGroup[r.group] = byGroup[r.group] || { pass: 0, fail: 0 }; byGroup[r.group][r.ok ? 'pass' : 'fail']++; });
  console.log('\nASSERTIONS');
  results.forEach(r => console.log(`${r.ok ? 'PASS' : 'FAIL'}  [${r.group}] ${r.name}`));
  console.log(`\nassertion totals: ${passed} passed, ${failed} failed of ${results.length}`);
  console.log('by group: ' + JSON.stringify(byGroup));

  // screenshots
  const snap = async name => { await page.waitForTimeout(350); await page.screenshot({ path: path.join(shots, name) }); };
  await page.click('#rev .sim [data-v="reset"]');
  await start('reminder'); await snap('01-combined-reminder.png');
  await start('setup'); await clickAction('suNext|'); await page.check('input[data-v="lan"]'); await page.waitForTimeout(200);
  await clickAction('suCheck|'); await page.waitForTimeout(700); await snap('02-setup-pairing.png');
  await start('weekly'); await clickAction('openReview|'); await clickAction('wkVar|mixed'); await snap('03-weekly-review-mixed.png');
  await start('monthly'); await snap('04-monthly-progress.png');
  await start('sharing'); await clickAction('shToggle|hydration'); await snap('05-sharing-preview.png');
  await start('sharing'); await click('#hyBody [data-a="shDelete"][data-v="hydration"]'); await click('#modal input[data-v="range"]'); await page.fill('#delFrom', '2026-10-12'); await page.fill('#delTo', '2026-10-16'); await click('#modal [data-a="shDelAsk"]'); await click('#modal [data-a="shDelGo"]'); await page.waitForTimeout(1100); await snap('07-delete-range-receipt.png');
  await start('camera'); await clickAction('camStart|allowed'); await page.waitForTimeout(800); await snap('06-camera-calibration.png');

  await browser.close();
  console.log(JSON.stringify({ assertions: { passed, failed, total: results.length }, flows: flows.length, perFlowFirstScreenActions: perFlow, clicks, keyboard: { focusedInBubble, bubbleClosed }, petLeftAfterDrag: left, networkRequests: requests, problems }, null, 2));
  process.exit(problems.length || requests.length || failed ? 1 : 0);
})();
