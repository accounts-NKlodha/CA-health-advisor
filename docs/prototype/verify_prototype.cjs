// Headless check for the simulated prototype (design review aid, not product code).
// Usage: NODE_PATH=<dir containing playwright> PLAYWRIGHT_BROWSERS_PATH=<browsers> node docs/prototype/verify_prototype.cjs
// Opens index.html from file://, clicks every flow in the reviewer index, clicks every visible action
// in each flow's first screen (restarting the flow each time), walks primary buttons, checks the
// keyboard path and pet drag, records console errors/warnings and any network request, saves screenshots,
// and runs targeted STATE assertions (revision 2: persistence and undo, range deletion, focus, contract wording, water copy, proposed defaults;
// revision 3, owner feedback 21 Oct: pet look, one reminder at a time, illustrated movement, visual weekly review, pet size, bring pet above clock).
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
  check('pace model: 11:15 with 250 ml logged is behind (expected 390 ml), so water is due', await ev(() => paceExpected() === 390 && behindPace() === true && S.acts.water.status === 'due'));
  check('reminder labels the sample scenario', /Sample scenario/.test(await ev(() => document.querySelector('#capBox').innerText)));
  await ev(() => { A.remResp('movement:done'); A.remAdv(5); A.remResp('eye:done'); A.remAdv(5); }); await page.waitForTimeout(100);
  const r1 = await bubbleText();
  check('water card (its own turn, 11:25) names the pace gap: 180 ml behind, about 430 ml expected', await ev(() => S.remActive === 'water') && /180 ml behind your own pace/.test(r1) && /about 430 ml expected/.test(r1));
  await ev(() => { F.reminder(); S.water = [{ id: 1, t: '09:50', ml: 500 }]; remTick(); A.remResp('movement:done'); A.remAdv(5); A.remResp('eye:done'); A.remAdv(10); }); await page.waitForTimeout(100);
  check('ahead of pace (500 ml at 11:15): water is not due and never shown', await ev(() => behindPace() === false && S.acts.water.status === 'notdue' && !document.querySelector('[data-rem]') && /On your pace/.test(document.querySelector('#capBox').innerText)));
  await ev(() => { F.reminder(); S.water = [{ id: 1, t: '09:50', ml: 2000 }]; remTick(); }); await page.waitForTimeout(100);
  check('goal reached: water paused and the status strip says so', await ev(() => S.acts.water.status === 'goal' && /Goal reached/.test(document.querySelector('#capBox').innerText)) && !/Water:/.test(await bubbleText()));
  await reset(); await start('pomodoro'); await ev(() => A.pomoStart('25/5')); await ev(() => A.pomoBreak()); await page.waitForTimeout(100);
  const pb1 = await bubbleText();
  check('pomoBreak behind pace: 500 ml logged at 15:00 vs about 1220 ml expected', /720 ml behind your own pace/.test(pb1) && /about 1220 ml expected/.test(pb1));
  await ev(() => { S.water = [{ id: 1, t: '10:00', ml: 1400 }]; A.pomoBreak(); }); await page.waitForTimeout(100);
  check('below daily total (1400 of 2000) but ahead of pace is NOT called behind', await ev(() => waterTotal() < S.waterGoal && behindPace() === false) && !/behind your own pace|Water:/.test(await bubbleText()));
  check('break card labels the sample scenario in the caption', /Sample scenario/.test(await ev(() => document.querySelector('#capBox').innerText)));

  // ---- Defaults approved by the owner on 21 Oct (labelled)
  G('6 proposed defaults');
  await reset(); await ev(() => settings('reminders'));
  check('presentation default carries the owner-approved label', await ev(() => /Owner approved default \(21 Oct\)/.test(document.querySelector('#hyBody').innerText) && /hidden completely/.test(document.querySelector('#hyBody').innerText) && /tray icon stays/.test(document.querySelector('#hyBody').innerText)));
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


  // ---------------------------------------------------------------------------------------------
  // Revision 3: owner feedback of 21 Oct (PR #4). One group per item; item 4 (deadline flow) is unchanged.
  // ---------------------------------------------------------------------------------------------
  const effOpacity = sel => ev(s2 => { let e = document.querySelector(s2), o = 1; while (e) { o *= +getComputedStyle(e).opacity; e = e.parentElement; } return Math.round(o * 1000) / 1000; }, sel);

  // ---- 7. Item 1: pet a little more transparent; messages and controls fully opaque
  G('7 pet look');
  await reset(); await start('desktop');
  const artOp = await ev(() => +getComputedStyle(document.querySelector('#petArt')).opacity);
  check(`pet body opacity is between 0.75 and 0.85 (is ${artOp})`, artOp >= 0.75 && artOp <= 0.85);
  check('pet menu bubble is fully opaque (effective opacity 1 up the whole tree)', await effOpacity('#bubble') === 1 && await ev(() => document.querySelector('#bubble').classList.contains('show')));
  check('bubble buttons are fully opaque', await effOpacity('#bubble .btn') === 1);
  await ev(() => { closeBubble(); setPet('waiting', 'Movement waiting'); });
  check('state badge and text tag stay fully opaque while the body is translucent', await effOpacity('#petBadge') === 1 && await effOpacity('#petTag') === 1 && await effOpacity('#petArt') <= 0.85);
  await ev(() => A.hc());
  check('higher-contrast mode draws the pet solid (opacity 1)', await ev(() => +getComputedStyle(document.querySelector('#petArt')).opacity) === 1);
  await ev(() => A.hc());
  await start('petfix'); await click('#capBox [data-a="petLook"]');
  check('reviewer compare shows the previous opaque pet, then the revised one', await ev(() => +getComputedStyle(document.querySelector('#petArt')).opacity) === 1 && (await (async () => { await click('#capBox [data-a="petLook"]'); return ev(() => +getComputedStyle(document.querySelector('#petArt')).opacity); })()) === artOp);

  // ---- 8. Item 2: one of movement / eye rest / water at a time, each with its own response and status
  G('8 one reminder at a time');
  await reset(); await start('reminder');
  await ev(() => { window.__maxRem = 0; const o = new MutationObserver(() => { window.__maxRem = Math.max(window.__maxRem, document.querySelectorAll('[data-rem]').length); }); o.observe(document.body, { subtree: true, childList: true, attributes: true }); window.__remObs = o; });
  const remCount = () => ev(() => [...document.querySelectorAll('[data-rem]')].filter(e => e.offsetParent !== null).length);
  const st = () => ev(() => JSON.parse(JSON.stringify(S.acts)));
  check('flow opens with exactly ONE reminder card (movement first)', await remCount() === 1 && await ev(() => S.remActive === 'movement' && !!document.querySelector('#bubble [data-rem="movement"]')));
  check('status strip shows each activity with its own status', await ev(() => { const r = [...document.querySelectorAll('#bubble .remStrip .rs')]; return r.length === 3 && r.map(x => x.dataset.k).join() === 'movement,eye,water' && r[0].dataset.status === 'showing' && r[1].dataset.status === 'due' && r[2].dataset.status === 'due'; }));
  check('each card has its own responses (movement: start, already moved, snooze, skip)', await ev(() => ['movement:done', 'movement:snooze', 'movement:skip'].every(v => document.querySelector(`#bubble [data-a="remResp"][data-v="${v}"]`)) && !!document.querySelector('#bubble [data-a="moveFromRem"]')));
  await click('#bubble [data-a="remResp"][data-v="movement:done"]');
  let s1 = await st();
  check('answering movement marks only movement done; eye rest and water stay due', s1.movement.status === 'done' && s1.eye.status === 'due' && s1.water.status === 'due');
  check('no burst: no other reminder appears straight after an answer', await remCount() === 0);
  await click('#capBox [data-a="remAdv"][data-v="1"]');
  check('calm gap: 1 min later still no reminder', await remCount() === 0 && await ev(() => S.remActive === null));
  await ev(() => A.remAdv(4)); await page.waitForTimeout(100);
  check('after the 5-min calm gap exactly one reminder (eye rest) appears', await remCount() === 1 && await ev(() => S.remActive === 'eye' && !!document.querySelector('#bubble [data-rem="eye"]')));
  await click('#bubble [data-a="remResp"][data-v="eye:snooze"]');
  const s2 = await st();
  check('snoozing eye rest snoozes only eye rest (movement keeps its done record, water still due)', s2.eye.status === 'snoozed' && JSON.stringify(s2.movement) === JSON.stringify(s1.movement) && s2.water.status === 'due');
  await ev(() => A.remAdv(5)); await page.waitForTimeout(100);
  check('next calm gap: water appears on its own', await remCount() === 1 && await ev(() => S.remActive === 'water' && !!document.querySelector('#bubble [data-rem="water"]')));
  const beforeW = await st(), mlBefore = await ev(() => waterTotal());
  await click('#bubble [data-a="remResp"][data-v="water:done:250"]');
  const afterW = await st();
  check('responding to WATER leaves movement and eye-rest status exactly unchanged', JSON.stringify(afterW.movement) === JSON.stringify(beforeW.movement) && JSON.stringify(afterW.eye) === JSON.stringify(beforeW.eye) && afterW.movement.status === 'done' && afterW.eye.status === 'snoozed');
  check('water is recorded as done and 250 ml is logged', afterW.water.status === 'done' && await ev(() => waterTotal()) === mlBefore + 250);
  await ev(() => A.remAdv(5)); await page.waitForTimeout(100);
  check('snoozed eye rest returns once at 11:30, on its own', await remCount() === 1 && await ev(() => S.remActive === 'eye' && S.clock.endsWith('11:30')));
  await click('#bubble [data-a="remResp"][data-v="eye:skip"]');
  const s3 = await st();
  check('Skip is recorded as skipped, never as done, and touches nothing else', s3.eye.status === 'skipped' && s3.movement.status === 'done' && s3.water.status === 'done');
  check('never more than one reminder card in the DOM during the whole sequence', await ev(() => window.__maxRem) === 1);
  await ev(() => A.remAdv(30)); await page.waitForTimeout(100);
  const k30 = await ev(() => S.remActive);
  await ev(() => A.closeBubble()); await page.waitForTimeout(80);
  check('closing a card without answering folds it into a badge (waiting), no other card shows', await ev(k => S.acts[k].status === 'waiting' && S.mode === 'waiting', k30) && await remCount() === 0 && await ev(() => window.__maxRem) === 1);
  await ev(() => { onPetClick(); }); 
  check('clicking the pet reopens that same reminder', await ev(k => S.remActive === k && !!document.querySelector(`#bubble [data-rem="${k}"]`), k30));
  await ev(() => window.__remObs.disconnect());
  await start('reminder'); await click('#bubble [data-a="moveFromRem"]'); await click('#bubble [data-a="mvVer"][data-v="seated"]');
  check('starting the routine from the movement card shows movement "In progress" only', await ev(() => S.acts.movement.status === 'active' && S.acts.eye.status === 'due' && S.acts.water.status === 'due'));
  const since0 = await ev(() => S.acts.movement.since);
  await click('#bubble [data-a="mvStop"]');
  check('immediate Stop from a reminder records none (not partial), keeps the movement clock; eye rest and water untouched', await ev(s0 => S.acts.movement.status === 'none' && S.acts.movement.since === s0 && S.acts.eye.status === 'due' && S.acts.water.status === 'due' && S.remActive === null && S.gapUntil > nowMin(), since0) && /Stopped, none recorded/.test(await bubbleText()));
  const mvCase = async (seq) => { await start('reminder'); await click('#bubble [data-a="moveFromRem"]'); await click('#bubble [data-a="mvVer"][data-v="seated"]'); const s0 = await ev(() => S.acts.movement.since); await ev(() => { S._t0 = nowMin(); }); for (const a of seq) await click(`#bubble [data-a="${a}"]`); return { st: await ev(() => S.acts.movement.status), since: await ev(() => S.acts.movement.since), s0, now: await ev(() => nowMin()) }; };
  const nd0 = await ev(() => ({ due: S.acts.movement.due, since: S.acts.movement.since, gap: S.gapUntil }));
  check('after none, no movement reminder returns inside the 5-minute calm gap', await ev(() => { A.remAdv(4); return S.remActive !== 'movement' && S.acts.movement.status === 'none'; }));
  check('after none, advancing past the next due time brings the movement reminder back; a.since still unchanged', await ev(d => { A.remAdv(Math.max(1, d.due - nowMin() + 1)); for (let i = 0; i < 30 && S.remActive !== 'movement'; i++) { if (S.remActive) remRecord(S.remActive, 'snooze'); A.remAdv(1); } return (S.acts.movement.status === 'showing' || S.remActive === 'movement') && S.acts.movement.since === d.since; }, nd0) && await ev(() => !!document.querySelector('#bubble [data-rem="movement"]')));
  await click('#bubble [data-a="remResp"][data-v="movement:done"]');
  check('a.since changes only on self-reported movement (Already moved)', await ev(d => S.acts.movement.since !== d.since && S.acts.movement.status === 'done', nd0));
  let mc = await mvCase(['mvSkip', 'mvSkip', 'mvSkip']);
  check('all three skipped from a reminder records none and does not reset the movement clock', mc.st === 'none' && mc.since === mc.s0);
  mc = await mvCase(['mvDone', 'mvStop']);
  check('one Done then Stop records partial and resets the movement clock', mc.st === 'partial' && mc.since === mc.now);
  mc = await mvCase(['mvDone', 'mvDone', 'mvDone']);
  check('all three Done records done and resets the movement clock', mc.st === 'done' && mc.since === mc.now);
  check('Reminders tab explains one reminder at a time with a calm gap', await (async () => { await ev(() => settings('reminders')); return /One reminder at a time/.test(await ev(() => document.querySelector('#hyBody').innerText)); })());

  // ---- 9. Item 3: movement routine says WHAT to do
  G('9 movement routine');
  await reset(); await start('movement');
  check('intro lists the three named movements with illustrations', await ev(() => document.querySelectorAll('#bubble .mvList li').length === 3 && document.querySelectorAll('#bubble .mvList svg.mvFig').length === 3));
  await click('#bubble [data-a="mvVer"][data-v="standing"]');
  const panel = () => ev(() => { const p = document.querySelector('#mvPanel'); if (!p) return null; return {
    key: p.dataset.mv, name: p.querySelector('.mvName')?.innerText.trim(), fig: !!p.querySelector('svg.mvFig[role="img"][aria-label]'), ghost: p.querySelectorAll('svg.mvFig .gh').length,
    steps: p.querySelectorAll('ol.mvSteps li').length, dose: p.querySelector('.mvDose')?.innerText || '', alt: p.querySelector('.mvAlt')?.innerText || '',
    skip: !!p.querySelector('[data-a="mvSkip"]'), stop: !!p.querySelector('[data-a="mvStop"]'), done: !!p.querySelector('[data-a="mvDone"]'), prov: /Provisional: pending content review/.test(p.innerText), text: p.innerText }; });
  const seen = [];
  for (let i = 0; i < 3; i++) {
    const pn = await panel(); seen.push(pn);
    check(`panel ${i + 1} (${pn && pn.name}): name, illustration, 3-5 numbered steps, dose, alternative, Skip and Stop, provisional label`, pn && pn.name && pn.fig && pn.ghost >= 1 && pn.steps >= 3 && pn.steps <= 5 && /(\d+\s?s\b|\d+ (slow )?(raises|rolls|times)|min)/.test(pn.dose) && /seated/i.test(pn.alt) && pn.skip && pn.stop && pn.done && pn.prov, JSON.stringify(pn && { ...pn, text: undefined }));
    if (i < 2) await click('#bubble [data-a="mvDone"]');
  }
  check('three different desk-safe movements shown (neck side stretch, seated spinal twist, standing calf raises)', seen.map(x => x.name).join('|') === 'Neck side stretch|Seated spinal twist|Standing calf raises');
  check('standing calf raises offer a seated alternative (seated heel raises)', /Seated alternative: seated heel raises/i.test(seen[2].alt));
  await click('#bubble [data-a="mvSkip"]');
  check('Skip on the last movement ends as partial (2 done, 1 skipped), not complete', /Saved as partial/.test(await bubbleText()) && /2 of 3 movements done, 1 skipped/.test(await bubbleText()));
  await start('movement'); await click('#bubble [data-a="mvVer"][data-v="seated"]'); await click('#bubble [data-a="mvDone"]'); await click('#bubble [data-a="mvDone"]');
  check('seated version swaps in the seated alternative (Seated heel raises) with its own figure', (await panel()).name === 'Seated heel raises' && (await panel()).fig);
  await click('#bubble [data-a="mvStop"]');
  check('Stop ends the routine and records partial', /stopped early/.test(await bubbleText()));
  const lib = await ev(() => Object.values(ROUTINES).map(r => ({ n: r.name, s: r.steps.length, a: !!r.alt, d: !!r.dose })));
  check(`library has at least 3 routines, each with 3-5 steps, dose and alternative (${lib.length})`, lib.length >= 3 && lib.every(r => r.s >= 3 && r.s <= 5 && r.a && r.d));
  check('no medical claims in movement content', await ev(() => !/cure|treat|prevent|relie|heal|therap|injur|posture correct|reduces? (pain|risk)/i.test(JSON.stringify(ROUTINES))));
  await start('movement'); await click('#bubble [data-a="mvVer"][data-v="standing"]');
  await page.emulateMedia({ reducedMotion: 'reduce' });
  check('reduced motion: figure animation stops, still start pose + dashed target pose remain', await ev(() => getComputedStyle(document.querySelector('#mvPanel .a-tilt')).animationName === 'none' && document.querySelectorAll('#mvPanel .gh').length >= 1));
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  check('with motion allowed the figure is animated', await ev(() => getComputedStyle(document.querySelector('#mvPanel .a-tilt')).animationName === 'mvTilt'));

  // ---- 10. Item 5: visual weekly review
  G('10 weekly review visual');
  await reset(); await start('weekly'); await click('#bubble [data-a="openReview"]');
  check('review renders three progress rings with one segment per workday', await ev(() => document.querySelectorAll('.wkRing > svg').length === 3 && [...document.querySelectorAll('.wkRing > svg')].every(s2 => s2.querySelectorAll('.rseg').length === 6)));
  check('week strip shows 7 days x 3 activities', await ev(() => document.querySelectorAll('.wkStrip .mk').length === 21 && document.querySelectorAll('.wkStrip .dh').length === 7));
  check('hero and concise highlights (2-3) are shown', await ev(() => !!document.querySelector('.wkHero .wkTitle') && document.querySelectorAll('.hi').length >= 2 && document.querySelectorAll('.hi').length <= 3));
  await click('#hyBody [data-a="wkVar"][data-v="missing"]');
  const nd = await ev(() => { const n = document.querySelector('.rseg.n'), z = document.querySelector('.rseg.z'), mn = document.querySelector('.wkStrip .mk.n'), mz = document.querySelector('.wkStrip .mk.z');
    return { nFill: n && n.getAttribute('fill'), zFill: z && z.getAttribute('fill'), zStroke: z && z.getAttribute('stroke'), nDash: n && n.getAttribute('stroke-dasharray'), mnTxt: mn && mn.innerText.trim(), mzTxt: mz && mz.innerText.trim(), mnBg: mn && getComputedStyle(mn).backgroundImage, mzBg: mz && getComputedStyle(mz).backgroundImage, legend: document.querySelector('#wkLegend').innerText }; });
  check('"no data" ring segments are hatched with a dashed edge', /^url\(#h-/.test(nd.nFill || '') && !!nd.nDash, JSON.stringify(nd));
  check('a measured 0 is drawn differently: white segment with a coloured outline', nd.zFill === '#fff' && /^#/.test(nd.zStroke || '') && nd.zFill !== nd.nFill);
  check('week strip: no data shows "–" on hatching, 0 shows "0" on plain white', nd.mnTxt === '–' && nd.mzTxt === '0' && /repeating-linear-gradient/.test(nd.mnBg) && nd.mzBg === 'none');
  check('legend explains no data (app not running) vs 0 (app running)', /no data: app not running/.test(nd.legend) && /0: none recorded/.test(nd.legend) && !/not done/.test(nd.legend));
  check('table view copy says no completion or amount was logged', await ev(() => /no completion or amount was logged/.test(document.documentElement.innerHTML) && !/nothing was done or logged/.test(document.documentElement.innerHTML)));
  await click('#hyBody [data-a="wkVar"][data-v="mixed"]');
  await click('#hyBody [data-a="sug"][data-v="afternoon"]');
  check('suggestion Apply still bound to state in the new layout (afternoon 16:00)', await ev(() => S.afternoonAt === '16:00' && /Applied/.test(document.querySelector('#sug-afternoon').innerText)));
  await click('#hyBody [data-a="sugUndo"][data-v="afternoon"]');
  check('Undo restores the previous value', await ev(() => S.afternoonAt === '15:00' && !S.sugApplied.afternoon));
  await page.emulateMedia({ reducedMotion: 'reduce' });
  check('prefers-reduced-motion: ring and strip animations are off', await ev(() => getComputedStyle(document.querySelector('.rseg')).animationName === 'none' && getComputedStyle(document.querySelector('.mk')).animationName === 'none'));
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  check('with motion allowed the rings animate in', await ev(() => getComputedStyle(document.querySelector('.rseg')).animationName === 'segIn'));
  await ev(() => A.rm());
  check('the in-app Reduced motion setting also stops the animations', await ev(() => getComputedStyle(document.querySelector('.rseg')).animationName === 'none'));
  await ev(() => A.rm()); await ev(() => A.openReview());
  await click('#hyBody [data-a="wkPlain"]');
  check('"Show as table" gives a plain readable view: table, no rings, toggle pressed', await ev(() => !!document.querySelector('#wkTable') && !document.querySelector('.wkRing') && document.querySelector('[data-a="wkPlain"]').getAttribute('aria-pressed') === 'true'));
  check('table view keeps "no data" distinct from "0" and lists what went well', await (async () => { await click('#hyBody [data-a="wkVar"][data-v="missing"]'); const t = await ev(() => document.querySelector('#wkTable').innerText); return /no data/.test(t) && /\t0\t|\b0\b/.test(t) && /What went well/.test(await ev(() => document.querySelector('#hyBody').innerText)); })());
  await click('#hyBody [data-a="wkVar"][data-v="mixed"]');
  await click('#hyBody [data-a="sug"][data-v="serving"]');
  check('suggestions work in the table view too (750 ml button applied, then undone)', await ev(() => S.servings.includes(750)) && await (async () => { await click('#hyBody [data-a="sugUndo"][data-v="serving"]'); return ev(() => S.servings.join() === '150,250,500'); })());
  await click('#hyBody [data-a="wkPlain"]');
  check('toggle returns to the graphics', await ev(() => !!document.querySelector('.wkRing') && !document.querySelector('#wkTable')));

  // ---- 11. Item 6: pet size control
  G('11 pet size');
  await reset(); await start('desktop');
  const petBox = () => ev(() => { const p = document.querySelector('#pet'); return { w: p.offsetWidth, h: p.offsetHeight, bottom: p.offsetTop + p.offsetHeight }; });
  const b0 = await petBox();
  check('pet menu has a size control bound to S.scale (Medium pressed)', b0.w === 128 && await ev(() => document.querySelector('#bubble [data-a="petSize"][data-v="100"]').getAttribute('aria-pressed') === 'true'));
  await click('#bubble [data-a="petSize"][data-v="125"]');
  const b1 = await petBox();
  check('choosing Large in the pet menu resizes the pet live (128 -> 160 px)', b1.w === 160 && b1.h === 160 && await ev(() => S.scale === 125));
  check('resizing keeps the pet\'s bottom edge in place', b1.bottom === b0.bottom);
  check('pet menu shows Large pressed after re-render', await ev(() => document.querySelector('#bubble [data-a="petSize"][data-v="125"]').getAttribute('aria-pressed') === 'true'));
  await ev(() => A.settings('pet'));
  check('Settings > Pet shows the same size (Large chip + slider 125)', await ev(() => document.querySelector('#hyBody [data-a="petSize"][data-v="125"]').getAttribute('aria-pressed') === 'true' && document.querySelector('#petScale').value === '125'));
  check('Settings notes the size control is required in the live product', /Required in the live product/.test(await ev(() => document.querySelector('#hyBody').innerText)));
  await click('#hyBody [data-a="petSize"][data-v="75"]');
  check('Small in Settings resizes the pet live (96 px)', (await petBox()).w === 96);
  await page.$eval('#petScale', el => { el.value = 150; el.dispatchEvent(new Event('input', { bubbles: true })); });
  check('slider (75-150 %) resizes the pet to 192 px and updates the chips', (await petBox()).w === 192 && await ev(() => document.querySelector('#hyBody [data-a="petSize"][data-v="150"]').getAttribute('aria-pressed') === 'true'));
  await ev(() => { closeWin(); petMenu(); });
  check('size persists on reopen: pet menu shows XL', await ev(() => document.querySelector('#bubble [data-a="petSize"][data-v="150"]').getAttribute('aria-pressed') === 'true'));
  await ev(() => A.settings('reminders')); await ev(() => A.settings('pet'));
  check('size persists on reopening Settings (slider 150)', await ev(() => document.querySelector('#petScale').value === '150' && S.scale === 150));
  await ev(() => { S.flow = 'petfix'; A.simRestart(); });
  check('size persists after a simulated app restart (192 px)', (await petBox()).w === 192);

  // ---- 12. Item 7: Bring pet above clock (tray menu and Settings), after a lost pet and after display changes
  G('12 bring pet above clock');
  const geom = () => ev(() => { const r = s2 => { const b = document.querySelector(s2).getBoundingClientRect(); return { l: b.left, t: b.top, r: b.right, b: b.bottom, w: b.width }; };
    return { pet: r('#pet'), desk: r('#desk'), clock: r('#clock'), bar: r('#taskbar'), shown: document.querySelector('#pet').style.display !== 'none' }; });
  const inside = g => g.shown && g.pet.l >= g.desk.l - 0.5 && g.pet.r <= g.desk.r + 0.5 && g.pet.t >= g.desk.t - 0.5 && g.pet.b <= g.desk.b + 0.5;
  const above = g => { const cx = (g.clock.l + g.clock.r) / 2; return g.pet.b <= g.clock.t + 0.5 && g.clock.t - g.pet.b < 40 && g.pet.l <= cx && cx <= g.pet.r; };
  const trayBring = async () => { await click('#trayHY'); await click('#trayMenu [data-a="trayDo"][data-v="bring"]'); await page.waitForTimeout(80); };
  const settingsBring = async () => { await ev(() => A.settings('pet')); await click('#bringCard [data-a="bringPet"]'); await page.waitForTimeout(80); };
  await reset(); await start('petfix');
  check('tray menu offers "Bring pet above clock"', await (async () => { await click('#trayHY'); const ok = await ev(() => /Bring pet above clock/.test(document.querySelector('#trayMenu').innerText)); await click('#trayHY'); return ok; })());
  await click('#capBox [data-a="simLost"]');
  check('simulated lost pet is outside the work area', !inside(await geom()));
  await trayBring();
  let g = await geom();
  check('from the TRAY: lost pet returns directly above the clock', above(g), JSON.stringify(g));
  check('from the TRAY: pet bounding box is inside the visible work area', inside(g), JSON.stringify(g));
  await ev(() => A.simLost()); await settingsBring(); g = await geom();
  check('from SETTINGS: lost pet returns above the clock and inside the work area', above(g) && inside(g), JSON.stringify(g));
  await ev(() => closeWin()); await ev(() => A.simDisplay('small')); g = await geom();
  check('display change to 1024 x 640 strands the pet outside the new work area', !inside(g) && g.desk.r < 1100);
  await trayBring(); g = await geom();
  check('after the display change, TRAY bring puts the pet above the clock inside the smaller work area', above(g) && inside(g), JSON.stringify(g));
  await ev(() => A.simLost()); await settingsBring(); g = await geom();
  check('after the display change, SETTINGS bring also works (above clock, inside)', above(g) && inside(g), JSON.stringify(g));
  await ev(() => { closeWin(); S.scale = 150; drawPet(); A.simLost(); }); await trayBring(); g = await geom();
  check('Extra-large pet is still placed fully inside the smaller work area, above the clock', above(g) && inside(g) && g.pet.w === 192, JSON.stringify(g));
  await ev(() => { S.scale = 100; drawPet(); A.simDisplay('normal'); A.simDisplay('right'); }); g = await geom();
  check('taskbar moved to the right leaves the pet partly outside the work area', !inside(g));
  await trayBring(); g = await geom();
  check('taskbar on the right: bring puts the pet inside the work area, beside the clock (bottom-right, next to the taskbar)', inside(g) && g.pet.r <= g.bar.l + 0.5 && g.pet.b > g.clock.t && g.pet.r > g.desk.r - 30, JSON.stringify(g));
  await ev(() => A.simLost()); await settingsBring(); g = await geom();
  check('taskbar on the right: SETTINGS bring also recovers the pet inside the work area', inside(g) && g.pet.r <= g.bar.l + 0.5, JSON.stringify(g));
  await ev(() => { closeWin(); A.simDisplay('normal'); A.simLost(); }); await trayBring(); g = await geom();
  check('display restored: bring returns the pet above the clock at full resolution', above(g) && inside(g) && g.desk.r > 1300, JSON.stringify(g));

  // ---- 13. Kept as they were
  G('13 unchanged decisions');
  await reset(); await ev(() => settings('reminders'));
  const prop = [await ev(() => document.querySelectorAll('#hyBody .proposed').length)];
  await ev(() => settings('camera')); prop.push(await ev(() => document.querySelectorAll('#hyBody .proposed').length));
  check('the three owner-approved default labels are shown (2 on Reminders, 1 on Posture check)', prop[0] === 2 && prop[1] === 1 && (src.match(/Owner approved default \(21 Oct\)/g) || []).length >= 2 && !/Proposed default, owner to confirm/.test(src));
  check('approved defaults are labelled as the owner decision of 21 Oct, with no pending label left', /Owner approved default \(21 Oct\)/.test(src) && !/owner to confirm/i.test(src.replace(/7 days[^<]{0,80}/g, '')));
  await start('admin');
  check('per-user restore rule with the 7-day maximum is unchanged (owner decision)', /7 days is the maximum wait \(owner decision, 21 Oct\)/.test(await ev(() => document.querySelector('#adminBody').innerText)));
  await start('recovery');
  check('deadline / recovery-day flow unchanged (check-in offered with Yes / No)', await ev(() => !!document.querySelector('#bubble [data-a="recYes"]') && !!document.querySelector('#bubble [data-a="recNo"]')));
  check('simulation banner and Draft labels intact', await ev(() => document.querySelector('#banner').innerText === 'Simulated prototype for design review. Not the product. No data is saved or sent.') && /Draft pending contract/.test(src));

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
  const slow = async name => { await page.waitForTimeout(1500); await page.screenshot({ path: path.join(shots, name) }); };
  await start('weekly'); await clickAction('openReview|'); await clickAction('wkVar|mixed'); await slow('03-weekly-review-mixed.png');
  await start('monthly'); await snap('04-monthly-progress.png');
  await start('sharing'); await clickAction('shToggle|hydration'); await snap('05-sharing-preview.png');
  await start('sharing'); await click('#hyBody [data-a="shDelete"][data-v="hydration"]'); await click('#modal input[data-v="range"]'); await page.fill('#delFrom', '2026-10-12'); await page.fill('#delTo', '2026-10-16'); await click('#modal [data-a="shDelAsk"]'); await click('#modal [data-a="shDelGo"]'); await page.waitForTimeout(1100); await snap('07-delete-range-receipt.png');
  await start('camera'); await clickAction('camStart|allowed'); await page.waitForTimeout(800); await snap('06-camera-calibration.png');
  // revision 3 screens
  await start('reminder'); await ev(() => { A.remResp('movement:done'); A.remAdv(5); A.remResp('eye:snooze'); A.remAdv(5); }); await snap('09-reminder-water-own-turn.png');
  await start('movement'); await slow('10-movement-routine-intro.png');
  await clickAction('mvVer|standing'); await slow('11-movement-routine-step.png');
  await clickAction('mvDone|'); await clickAction('mvDone|'); await slow('12-movement-seated-alternative.png');
  await start('weekly'); await clickAction('openReview|'); await clickAction('wkVar|strong'); await slow('13-weekly-review-strong.png');
  await clickAction('wkVar|missing'); await slow('14-weekly-review-no-data-vs-zero.png');
  await clickAction('wkVar|mixed'); await clickAction('wkPlain|'); await slow('15-weekly-review-table-view.png'); await clickAction('wkPlain|');
  await start('petfix'); await ev(() => A.petSize(125)); await ev(() => A.settings('pet')); await ev(() => { document.querySelector('#hyBody .content').scrollTop = 330; }); await snap('16-pet-size-settings.png');
  await start('petfix'); await ev(() => A.petSize(125)); await snap('17-pet-menu-size-and-look.png');
  await page.screenshot({ path: path.join(shots, '18-pet-look-revised-detail.png'), clip: { x: 820, y: 330, width: 546, height: 438 } });
  await ev(() => A.petLook()); await page.waitForTimeout(200); await page.screenshot({ path: path.join(shots, '19-pet-look-previous-detail.png'), clip: { x: 820, y: 330, width: 546, height: 438 } }); await ev(() => A.petLook());
  await ev(() => A.petSize(100)); await ev(() => A.simDisplay('small')); await click('#trayHY'); await snap('20-pet-lost-after-display-change.png');
  await click('#trayMenu [data-a="trayDo"][data-v="bring"]'); await snap('21-bring-pet-above-clock.png');
  await ev(() => { A.simDisplay('right'); }); await click('#trayHY'); await click('#trayMenu [data-a="trayDo"][data-v="bring"]'); await snap('22-bring-pet-taskbar-right.png');
  await ev(() => A.simDisplay('normal'));

  await browser.close();
  console.log(JSON.stringify({ assertions: { passed, failed, total: results.length }, flows: flows.length, perFlowFirstScreenActions: perFlow, clicks, keyboard: { focusedInBubble, bubbleClosed }, petLeftAfterDrag: left, networkRequests: requests, problems }, null, 2));
  process.exit(problems.length || requests.length || failed ? 1 : 0);
})();
