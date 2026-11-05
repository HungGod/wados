// A browser on the stream: headless Chromium (Playwright) opens Selkies'
// page, waits for frames, saves a screenshot, clicks the wadspace's Start
// button (input), and counts the Opus audio frames the WebSocket brings
// each second.
//
//   node tools/krun/view.mjs URL OUT.png [SECONDS]
//
// TRIGGER_CMD (with TRIGGER_AFTER seconds, default 6): a shell command run
// part-way through (prototype.sh: the guest's tone), so the audio counts
// show silence, then sound.
// PLAYWRIGHT_CORE: a playwright-core folder (default: the v2 spike's, whose
// Chromium is in ~/.cache/ms-playwright). http://localhost is a secure
// context, so Selkies' WebCodecs decoder runs as it would behind wadd's HTTPS.
import { execSync } from "node:child_process";
import { createRequire } from "node:module";

const [url, out, secs = "20"] = process.argv.slice(2);
if (!url || !out) {
    console.error("usage: view.mjs URL OUT.png [SECONDS]");
    process.exit(2);
}
const core =
    process.env.PLAYWRIGHT_CORE ??
    new URL("../../../wadspaces-v2-spike/node_modules/playwright-core", import.meta.url).pathname;
const { chromium } = createRequire(import.meta.url)(core);

const browser = await chromium.launch({ args: ["--autoplay-policy=no-user-gesture-required"] });
const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
const log = [];
const t0 = Date.now();
// Per second since load: [audio frames, audio bytes, other binary frames].
const perSec = [];
page.on("console", (m) => log.push(`[${m.type()}] ${m.text()}`));
page.on("websocket", (ws) => {
    log.push(`[ws] ${ws.url()}`);
    ws.on("framereceived", ({ payload }) => {
        if (typeof payload === "string") return;
        const s = Math.floor((Date.now() - t0) / 1000);
        perSec[s] ??= [0, 0, 0];
        if (payload[0] === 0x01) {
            perSec[s][0]++;
            perSec[s][1] += payload.length;
        } else {
            perSec[s][2]++;
        }
    });
});
await page.goto(url, { waitUntil: "domcontentloaded" });
// Selkies draws onto a canvas (WebCodecs) once the WebSocket carries frames.
let drawn = false;
for (let i = 0; i < 60 && !drawn; i++) {
    await page.waitForTimeout(500);
    drawn = await page.evaluate(() =>
        [...document.querySelectorAll("canvas")].some((e) => e.width > 64 && e.height > 64),
    );
}
const firstFrame = (Date.now() - t0) / 1000;
// A click in the middle: the page's "start" overlay (audio needs a gesture), and focus.
await page.mouse.click(640, 400);
await page.waitForTimeout(3000);
await page.screenshot({ path: out });
// The wadspace's Start button (selkies-desktop's panel, bottom left).
await page.mouse.click(30, 772);
await page.waitForTimeout(2500);
await page.screenshot({ path: out.replace(/\.png$/, "-start.png") });
await page.keyboard.press("Escape");

const trigger = process.env.TRIGGER_CMD;
const after = Number(process.env.TRIGGER_AFTER ?? 6);
let triggeredAt = null;
const end = Date.now() + Number(secs) * 1000;
while (Date.now() < end) {
    await page.waitForTimeout(500);
    if (trigger && triggeredAt === null && Date.now() - t0 > (firstFrame + after) * 1000) {
        triggeredAt = (Date.now() - t0) / 1000;
        execSync(trigger, { stdio: "inherit" });
    }
}
await page.screenshot({ path: out.replace(/\.png$/, "-end.png") });
const state = await page.evaluate(() => ({
    title: document.title,
    surfaces: [...document.querySelectorAll("canvas, video")].map((e) => `${e.tagName} ${e.width}x${e.height}`),
}));
console.log(JSON.stringify({ url, drawn, firstFrame, triggeredAt, ...state }));
console.log("second: audio frames/bytes, video frames");
console.log(
    perSec
        .map((v, s) => (v ? `${s}s: ${v[0]}/${v[1]}B, ${v[2]}` : null))
        .filter(Boolean)
        .join(" | "),
);
console.log(log.filter((l) => /error|audio|Stream started/i.test(l)).slice(-12).join("\n"));
await browser.close();
