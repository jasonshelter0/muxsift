// Optional browser QA: NODE_PATH=/tmp/muxsift-gui-qa/node_modules node prototypes/gui/qa.cjs
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const { pathToFileURL } = require('node:url');
const base = process.env.GUI_BASE_URL || pathToFileURL(__dirname + '/').href;
(async () => {
  const browser = await chromium.launch({ headless: true });
  try {
    for (const theme of ['liquid-glass', 'neo-brutalism']) {
      const page = await browser.newPage({ viewport: { width: 1440, height: 1100 }, acceptDownloads: true });
      const errors = [], remoteRequests = [];
      page.on('pageerror', e => errors.push(e.message));
      page.on('request', r => { if (!r.url().startsWith(base) && !r.url().startsWith('blob:')) remoteRequests.push(r.url()); });
      await page.clock.install();
      await page.goto(base + theme + '.html');
      assert.equal(await page.locator('.task-row').count(), 3);
      await page.screenshot({ path: path.join(__dirname, 'previews', theme + '.png'), fullPage: true });
      await page.locator('[data-action="create"]').click();
      assert.equal(await page.locator('#input-error').isVisible(), true);
      await page.locator('[data-action="sample"]').click();
      await page.locator('[data-action="create"]').click();
      assert.equal(await page.locator('.task-row').count(), 4);
      await page.clock.runFor(1300);
      await page.locator('.row-action[data-action="pause"]').first().click();
      const paused = await page.locator('.task-row.selected').innerText();
      assert.ok(paused.includes('已暂停'));
      await page.clock.runFor(2000);
      assert.equal(await page.locator('.task-row.selected').innerText(), paused);
      await page.locator('.task-row.selected .row-action').click();
      await page.clock.runFor(14000);
      assert.ok((await page.locator('.task-row.selected').innerText()).includes('已完成'));
      const downloadEvent = page.waitForEvent('download');
      await page.locator('[data-action="export"]').click();
      const download = await downloadEvent;
      assert.equal(download.suggestedFilename(), 'muxsift-demo.md');
      assert.ok((await fs.readFile(await download.path(), 'utf8')).includes('未进行真实 ASR'));
      await page.locator('[data-preview="frames"]').click();
      assert.equal(await page.locator('.frame-card').count(), 3);
      await page.locator('.frame-card').nth(1).click();
      assert.equal(await page.locator('.note-paper').isVisible(), true);
      await page.locator('[data-preview="timeline"]').click();
      assert.equal(await page.locator('.timeline-item').count(), 4);
      await page.locator('#search').fill('does-not-exist');
      assert.equal(await page.locator('.task-row').count(), 0);
      await page.locator('#search').fill('');
      await page.locator('.nav-item[data-action="settings"]').click();
      await page.locator('select[name="asr"]').selectOption('bcut');
      await page.locator('select[name="format"]').selectOption('srt');
      await page.locator('#settings-form button[type="submit"]').click();
      assert.equal(await page.locator('dialog').isVisible(), false);
      await page.reload();
      assert.ok((await page.locator('#engine-label').innerText()).includes('Bcut'));
      assert.equal(await page.locator('[data-mode="srt"]').getAttribute('aria-pressed'), 'true');
      await page.locator('.nav-item[data-action="settings"]').click();
      await page.keyboard.press('Escape');
      assert.equal(await page.locator('dialog').isVisible(), false);
      await page.locator('[data-source="local"]').click();
      await page.locator('#file-input').setInputFiles([
        { name: 'lecture.mp4', mimeType: 'video/mp4', buffer: Buffer.from('demo') },
        { name: 'podcast.wav', mimeType: 'audio/wav', buffer: Buffer.from('demo') },
      ]);
      assert.ok((await page.locator('#file-label').innerText()).includes('2 个文件'));
      await page.locator('[data-action="create"]').click();
      assert.equal(await page.locator('.task-row').count(), 5);
      await page.clock.runFor(14000);
      assert.equal(await page.locator('.subtitle-text').isVisible(), true);
      const srtEvent = page.waitForEvent('download');
      await page.locator('[data-action="export"]').click();
      assert.equal((await srtEvent).suggestedFilename(), 'muxsift-demo.srt');
      await page.locator('[data-view="library"]').click();
      assert.equal(await page.locator('.composer').isVisible(), false);
      assert.equal(await page.locator('.task-row').count(), 4);
      await page.locator('[data-action="reset"]').click();
      assert.equal(await page.locator('.composer').isVisible(), true);
      await page.locator('[data-mode="frames"]').click();
      await page.locator('[data-source="local"]').click();
      await page.locator('#file-input').setInputFiles({ name: 'audio.mp3', mimeType: 'audio/mpeg', buffer: Buffer.from('demo') });
      await page.locator('[data-action="create"]').click();
      assert.ok((await page.locator('#input-error').innerText()).includes('需要视频文件'));
      await page.locator('[data-source="url"]').click();
      await page.locator('[data-action="sample"]').click();
      await page.locator('[data-action="create"]').click();
      await page.clock.runFor(14000);
      const jsonEvent = page.waitForEvent('download');
      await page.locator('[data-action="export"]').click();
      const jsonDownload = await jsonEvent;
      assert.equal(jsonDownload.suggestedFilename(), 'muxsift-demo.json');
      assert.equal(JSON.parse(await fs.readFile(await jsonDownload.path(), 'utf8')).demo, true);
      // Restore initial settings so screenshots represent the default candidates.
      await page.evaluate(() => localStorage.clear());
      await page.reload();
      for (const width of [320, 390, 768, 1024, 1440]) {
        await page.setViewportSize({ width, height: 900 });
        const sizes = await page.evaluate(() => ({ scroll: document.documentElement.scrollWidth, inner: innerWidth }));
        assert.ok(sizes.scroll <= sizes.inner, `${theme}: horizontal overflow at ${width}: ${sizes.scroll}`);
        await page.evaluate(() => document.activeElement?.blur());
        if (width === 390) await page.screenshot({ path: path.join(__dirname, 'previews', theme + '-mobile.png'), fullPage: true });
      }
      assert.deepEqual(errors, []);
      assert.deepEqual(remoteRequests, []);
      console.log(`${theme}: input, simulation, pause/resume, Markdown/SRT export, preview, settings, local batch, library, 5 viewport sizes PASS`);
      await page.close();
    }
    const page = await browser.newPage();
    await page.goto(base + 'index.html');
    assert.equal(await page.locator('.candidate').count(), 2);
    for (const img of await page.locator('img').all()) assert.ok(await img.evaluate(el => el.complete && el.naturalWidth > 0));
    await page.close();
    console.log('Comparison page: links and screenshot assets PASS');
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
