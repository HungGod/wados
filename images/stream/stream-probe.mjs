// Opens a stream the way a viewer does (headless Chromium) and says whether
// it streams: the page loads, and the stream's element (the largest canvas
// or video on it) shows the wadspace's desktop, its wallpaper's colour
// (--colour: try-stream.sh gives its wadspace one no page of Selkies' has)
// over most of it. Used by try-stream.sh.
//
//   node images/stream/stream-probe.mjs URL OUT.png --colour '#rrggbb'
//        [--expect stream|none] [--wait SECONDS]
//
// URL is the stream's page with its token, as a one-tap link carries it
// (http://127.0.0.1:<port>/ws-<guid>/#token=<key>), or without one. It
// prints one line of JSON (what it saw) and screenshots the page to OUT.png;
// with --expect it exits 1 unless it saw that ("none": the page loads, and
// nothing streams in the time given).
//
// Selkies' WebSocket transport decodes with WebCodecs, which needs a secure
// context: http://127.0.0.1 is one. Playwright comes from the spike's
// node_modules (the wadspaces-v2-spike checkout beside this one;
// WADOS_PLAYWRIGHT names another package.json beside a node_modules with
// playwright in it), and its Chromium from Playwright's cache.
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";

const args = process.argv.slice(2);
const opt = (name, fallback) => {
  const i = args.indexOf(name);
  if (i < 0) return fallback;
  const v = args[i + 1];
  args.splice(i, 2);
  return v;
};
const expect = opt("--expect", "");
const wait = Number(opt("--wait", "40"));
const colour = opt("--colour", "");
const [url, out] = args;
if (!url || !out || !/^#[0-9a-f]{6}$/i.test(colour) || !["", "stream", "none"].includes(expect)) {
  console.error("usage: stream-probe.mjs URL OUT.png --colour '#rrggbb' [--expect stream|none] [--wait SECONDS]");
  process.exit(2);
}
const rgb = [1, 3, 5].map((i) => parseInt(colour.slice(i, i + 2), 16));
const pkg =
  process.env.WADOS_PLAYWRIGHT ||
  fileURLToPath(new URL("../../../wadspaces-v2-spike/package.json", import.meta.url));
const { chromium } = createRequire(pkg)("playwright");

// The stream's element's picture: how much of it is the wallpaper's colour
// (within 24 a channel: the encoder's rounding), how many colours it has
// (quantized to 4 bits a channel) and how much its brightness varies.
async function picture(page, png) {
  return page.evaluate(async ([b64, rgb]) => {
    const els = [...document.querySelectorAll("canvas, video")]
      .map((e) => ({ e, r: e.getBoundingClientRect() }))
      .filter(({ r }) => r.width >= 64 && r.height >= 64)
      .sort((a, b) => b.r.width * b.r.height - a.r.width * a.r.height);
    if (!els.length) return { element: null };
    const { e, r } = els[0];
    const img = new Image();
    img.src = `data:image/png;base64,${b64}`;
    await img.decode();
    const scale = img.width / window.innerWidth;
    const [x, y] = [Math.max(0, r.left * scale), Math.max(0, r.top * scale)];
    const [w, h] = [Math.min(img.width - x, r.width * scale), Math.min(img.height - y, r.height * scale)];
    const c = new OffscreenCanvas(Math.max(1, Math.floor(w)), Math.max(1, Math.floor(h)));
    const g = c.getContext("2d");
    g.drawImage(img, x, y, w, h, 0, 0, c.width, c.height);
    const d = g.getImageData(0, 0, c.width, c.height).data;
    const colours = new Set();
    let n = 0, sum = 0, sq = 0, wallpaper = 0;
    for (let i = 0; i < d.length; i += 4 * 7) {
      colours.add(((d[i] >> 4) << 8) | ((d[i + 1] >> 4) << 4) | (d[i + 2] >> 4));
      if ([0, 1, 2].every((k) => Math.abs(d[i + k] - rgb[k]) <= 24)) wallpaper += 1;
      const l = 0.2126 * d[i] + 0.7152 * d[i + 1] + 0.0722 * d[i + 2];
      n += 1; sum += l; sq += l * l;
    }
    const mean = sum / n;
    return {
      element: `${e.tagName.toLowerCase()}${e.id ? "#" + e.id : ""}`,
      size: [Math.round(r.width), Math.round(r.height)],
      wallpaper: Math.round((wallpaper / n) * 100) / 100,
      colours: colours.size,
      stddev: Math.round(Math.sqrt(Math.max(0, sq / n - mean * mean)) * 10) / 10,
    };
  }, [png.toString("base64"), rgb]);
}
// Most of it is the wallpaper: the desktop is on it.
const shows = (pic) => Boolean(pic.element && pic.wallpaper >= 0.3);

const browser = await chromium.launch({ args: ["--autoplay-policy=no-user-gesture-required"] });
let seen = {};
try {
  const page = await (await browser.newContext({ viewport: { width: 1280, height: 800 } })).newPage();
  const sockets = [];
  page.on("websocket", (ws) => {
    const s = { url: ws.url().replace(/token=[^&]*/, "token=…"), frames: 0, closed: false };
    sockets.push(s);
    ws.on("framereceived", () => (s.frames += 1));
    ws.on("close", () => (s.closed = true));
  });
  const res = await page.goto(url, { waitUntil: "load", timeout: 30000 });
  const secure = await page.evaluate(() => window.isSecureContext);
  const rtc = () => page.evaluate(() => [...document.querySelectorAll("video")].some((v) => v.srcObject && v.readyState >= 2));
  let pic = {};
  const until = Date.now() + wait * 1000;
  for (;;) {
    await page.waitForTimeout(2000);
    pic = await picture(page, await page.screenshot());
    if (shows(pic) || Date.now() > until) break;
  }
  await page.screenshot({ path: out });
  const streaming = shows(pic);
  seen = {
    page: res ? res.status() : null,
    secure,
    streaming,
    video: await rtc(),
    ...pic,
    sockets,
    screenshot: out,
  };
} finally {
  await browser.close();
}
console.log(JSON.stringify(seen));
if (expect === "stream" && !(seen.page === 200 && seen.streaming)) process.exit(1);
if (expect === "none" && !(seen.page === 200 && !seen.streaming)) process.exit(1);
