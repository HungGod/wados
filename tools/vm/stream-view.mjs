// A viewer of a WadOS stream (headless Chromium, Playwright's), for
// tools/vm/check-stream.sh and check-tailnet.sh: it opens a stream's link
// as a person would, through wadd's proxy (a quick tunnel, or the tailnet),
// and says what it saw, as one line of JSON on stdout.
//
//   node tools/vm/stream-view.mjs < options.json
//
// Its options come on stdin (a stream's key and password never go on a
// command line), as JSON:
//   url        the link: the one-tap link (…/ws-<guid>/#token=<key>), or
//              the link without its key (the sign-in card)
//   out        where its screenshot goes (PNG)
//   expect     "stream": exit 1 unless it streams; "none": unless the page
//              loads and nothing streams; "refused": unless the sign-in
//              card refuses the password (403, "That’s not it")
//   wait       seconds to wait for it to stream (40)
//   hold       seconds to keep watching once it streams (0): a check reads
//              the stream's viewers meanwhile
//   user, password
//              signs in with them on the card (the plain link)
//   relay      true: WebRTC through TURN only (iceTransportPolicy "relay")
//   actions    once it streams, a person's input through Selkies (before the
//              hold): [{wait: ms}, {click: [x, y]}, {dblclick: [x, y]},
//              {type: "text"}, {press: "Enter"}], in page coordinates
//   switch     "websockets" or "webrtc": once signed in, the page asks
//              Selkies to switch (POST api/switch, with the cookie alone),
//              then loads again
//   proxy      a SOCKS5 proxy (socks5://127.0.0.1:11055: the tailnet's device)
//   secure     an origin to treat as secure (http://<tailnet ip>:<port>):
//              Selkies' WebSocket mode decodes with WebCodecs, which needs a
//              secure context. Only full Chromium honours it, so it runs
//              that (channel "chromium") rather than the headless shell.
//
// What it prints: the page's status; whether it's a secure context; whether
// it streams (its stream element shows something, not one colour, and
// frames keep coming); how (WebRTC: the candidate pair's types, from
// RTCPeerConnection's stats; or WebSockets); the ICE servers the page was
// given (their URLs only); the address bar (masked) and whether a token is
// still in it; the sockets, and on Selkies' data socket its frames by kind
// (the first byte: 1 audio, 3 and 4 video; audio comes only while there's
// sound); every 4xx/5xx it got and the console's errors.
// Tokens are masked (token=…) in all of it.
//
// Playwright comes from the spike's node_modules (wadspaces-v2-spike beside
// this checkout; WADOS_PLAYWRIGHT names another package.json).
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
import { readFileSync } from "node:fs";

const o = JSON.parse(readFileSync(0, "utf8"));
const expect = o.expect || "";
const wait = Number(o.wait ?? 40);
const hold = Number(o.hold ?? 0);
if (!o.url || !o.out || !["", "stream", "none", "refused"].includes(expect)) {
  console.error("stream-view.mjs: options on stdin: {url, out, expect?, wait?, hold?, user?, password?, relay?, proxy?, secure?}");
  process.exit(2);
}
const secrets = [o.password].filter(Boolean);
const mask = (s) => {
  let t = String(s ?? "").replace(/token=[^&#\s"']*/g, "token=…");
  for (const x of secrets) t = t.split(x).join("…");
  return t;
};
const pkg =
  process.env.WADOS_PLAYWRIGHT ||
  fileURLToPath(new URL("../../../wadspaces-v2-spike/package.json", import.meta.url));
const { chromium } = createRequire(pkg)("playwright");

// Every RTCPeerConnection the page makes, kept for its stats; relay: TURN only.
const init = (relay) => {
  const Orig = window.RTCPeerConnection;
  if (!Orig) return;
  window.__pcs = [];
  window.__ice = [];
  function PC(config, ...rest) {
    const c = Object.assign({}, config || {});
    if (relay) c.iceTransportPolicy = "relay";
    for (const s of c.iceServers || []) {
      for (const u of [].concat(s.urls || s.url || [])) window.__ice.push(String(u));
    }
    const pc = new Orig(c, ...rest);
    window.__pcs.push(pc);
    return pc;
  }
  PC.prototype = Orig.prototype;
  Object.setPrototypeOf(PC, Orig);
  window.RTCPeerConnection = PC;
  if (window.webkitRTCPeerConnection) window.webkitRTCPeerConnection = PC;
};

async function rtc(evaluate) {
  return evaluate(async () => {
    const out = [];
    for (const pc of window.__pcs || []) {
      let st;
      try {
        st = await pc.getStats();
      } catch (e) {
        continue;
      }
      const by = new Map();
      st.forEach((r) => by.set(r.id, r));
      let pair = null;
      st.forEach((r) => {
        if (r.type === "transport" && r.selectedCandidatePairId) pair = by.get(r.selectedCandidatePairId) || pair;
      });
      if (!pair) {
        st.forEach((r) => {
          if (!pair && r.type === "candidate-pair" && r.nominated && r.state === "succeeded") pair = r;
        });
      }
      const side = (c) => c && { type: c.candidateType, protocol: c.protocol, relayProtocol: c.relayProtocol, url: c.url };
      let frames = 0, bytes = 0, audio = 0;
      st.forEach((r) => {
        if (r.type === "inbound-rtp" && r.kind === "video") {
          frames += r.framesDecoded || 0;
          bytes += r.bytesReceived || 0;
        }
        if (r.type === "inbound-rtp" && r.kind === "audio") audio += r.packetsReceived || 0;
      });
      out.push({
        state: pc.connectionState,
        ice: pc.iceConnectionState,
        pair: pair && {
          local: side(by.get(pair.localCandidateId)),
          remote: side(by.get(pair.remoteCandidateId)),
          rtt: pair.currentRoundTripTime,
        },
        videoFrames: frames,
        videoBytes: bytes,
        audioPackets: audio,
      });
    }
    return { pcs: out, ice: [...new Set((window.__ice || []).map((u) => u.replace(/\?.*$/, "")))] };
  });
}

// The stream's element (the largest canvas or video): is anything on it?
async function picture(evaluate, png) {
  return evaluate(async (b64) => {
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
    let n = 0, sum = 0, sq = 0;
    for (let i = 0; i < d.length; i += 4 * 7) {
      colours.add(((d[i] >> 4) << 8) | ((d[i + 1] >> 4) << 4) | (d[i + 2] >> 4));
      const l = 0.2126 * d[i] + 0.7152 * d[i + 1] + 0.0722 * d[i + 2];
      n += 1;
      sum += l;
      sq += l * l;
    }
    const mean = sum / n;
    return {
      element: `${e.tagName.toLowerCase()}${e.id ? "#" + e.id : ""}`,
      size: [Math.round(r.width), Math.round(r.height)],
      colours: colours.size,
      stddev: Math.round(Math.sqrt(Math.max(0, sq / n - mean * mean)) * 10) / 10,
      visible: getComputedStyle(e).visibility !== "hidden" && getComputedStyle(e).display !== "none",
    };
  }, png.toString("base64"));
}

const launch = { args: ["--autoplay-policy=no-user-gesture-required"] };
if (o.proxy) launch.proxy = { server: o.proxy };
if (o.secure) {
  launch.args.push(`--unsafely-treat-insecure-origin-as-secure=${o.secure}`);
  launch.channel = "chromium";
}
if (process.env.STREAM_VIEW_CHANNEL) launch.channel = process.env.STREAM_VIEW_CHANNEL;
const browser = await chromium.launch(launch);
let seen = {};
let code = 0;
try {
  const page = await (await browser.newContext({ viewport: { width: 1280, height: 800 } })).newPage();
  await page.addInitScript(init, Boolean(o.relay));
  const sockets = [];
  const errors = [];
  const consoleErrors = [];
  page.on("websocket", (ws) => {
    const s = { url: mask(new URL(ws.url()).pathname + new URL(ws.url()).search), frames: 0, kinds: {}, closed: false };
    sockets.push(s);
    ws.on("framereceived", (f) => {
      s.frames += 1;
      if (typeof f.payload !== "string" && f.payload.length > 0) s.kinds[f.payload[0]] = (s.kinds[f.payload[0]] || 0) + 1;
    });
    ws.on("close", () => (s.closed = true));
  });
  page.on("response", (r) => {
    if (r.status() >= 400) errors.push(`${r.status()} ${r.request().method()} ${mask(new URL(r.url()).pathname)}`);
  });
  page.on("console", (m) => {
    if (m.type() === "error" && consoleErrors.length < 12) consoleErrors.push(mask(m.text()).slice(0, 200));
  });
  // The page may navigate under an evaluation (the one-tap card POSTs the
  // key, then loads the page again): wait for it to settle, then again.
  const settle = async () => {
    await page.waitForLoadState("load").catch(() => {});
    await page.waitForTimeout(500);
  };
  const ev = async (fn, arg) => {
    for (let i = 0; ; i++) {
      try {
        return await page.evaluate(fn, arg);
      } catch (e) {
        if (i >= 8 || !/context was destroyed|navigat/i.test(e.message)) throw e;
        await settle();
      }
    }
  };
  // The one-tap link: its card trades the key for the cookie (a POST, 204),
  // then loads the page again.
  const oneTap = /#token=/.test(o.url)
    ? page
        .waitForResponse((r) => r.request().method() === "POST" && /\/ws-[^/]+\/$/.test(new URL(r.url()).pathname), { timeout: 30000 })
        .then((r) => r.status())
        .catch(() => null)
    : null;
  const first = await page.goto(o.url, { waitUntil: "load", timeout: 45000 });
  seen.page = first ? first.status() : null;
  if (oneTap) {
    seen.oneTap = await oneTap;
    await page.waitForTimeout(1500);
    await settle();
  }
  // The plain link: the sign-in card, then the user and password.
  if (o.user !== undefined && o.password !== undefined) {
    await settle();
    const card = await page.$("form input[name=password]").catch(() => null);
    seen.card = Boolean(card);
    if (card) {
      await page.fill("form input[name=user]", o.user);
      await page.fill("form input[name=password]", o.password);
      const [nav] = await Promise.all([
        page.waitForNavigation({ waitUntil: "load", timeout: 30000 }).catch(() => null),
        page.click("form button[type=submit]"),
      ]);
      // The POST's answer (303 to the page, or the card again: 403, 429).
      seen.signIn = nav ? nav.status() : null;
      const posted = nav?.request()?.redirectedFrom()?.response?.();
      if (posted) seen.signInPost = (await posted)?.status?.() ?? null;
      const text = await ev(() => document.body?.innerText || "");
      seen.refused = /not it/i.test(text);
      seen.tooMany = seen.signIn === 429;
    }
  } else {
    seen.card = /is shared with you|Opening/.test(await ev(() => document.body?.innerText || ""));
  }
  // The mode switch, from the page with its cookie alone (no key in it).
  if (o.switch) {
    await settle();
    seen.switched = await ev(async (mode) => {
      const r = await fetch("api/switch", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ mode }),
        credentials: "same-origin",
      });
      return `${r.status} ${(await r.text()).slice(0, 80)}`;
    }, o.switch);
    await page.waitForTimeout(3000);
    await page.reload({ waitUntil: "load" }).catch(() => {});
    await settle();
  }
  seen.secure = await ev(() => window.isSecureContext);
  const progress = async () => {
    const r = await rtc(ev);
    const data = sockets.filter((s) => /api\/websockets/.test(s.url));
    const ws = data.reduce((a, s) => a + s.frames, 0);
    const audio = data.reduce((a, s) => a + (s.kinds[1] || 0), 0);
    const vf = r.pcs.reduce((a, p) => a + p.videoFrames, 0);
    const ra = r.pcs.reduce((a, p) => a + p.audioPackets, 0);
    return { r, ws, vf, audio, ra };
  };
  let pic = {};
  let p0 = null;
  let streaming = false;
  const until = Date.now() + (expect === "refused" ? 2 : wait) * 1000;
  for (;;) {
    await page.waitForTimeout(2000);
    pic = await picture(ev, await page.screenshot());
    const p = await progress();
    // It streams: something drawn on its element, and frames still coming.
    if (p0 && pic.element && pic.colours >= 3 && (p.vf > p0.vf || p.ws > p0.ws + 2)) {
      streaming = true;
      break;
    }
    p0 = p;
    if (Date.now() > until) break;
  }
  if (streaming && Array.isArray(o.actions)) {
    for (const a of o.actions) {
      if (a.wait) await page.waitForTimeout(a.wait);
      if (a.click) await page.mouse.click(a.click[0], a.click[1]);
      if (a.dblclick) await page.mouse.dblclick(a.dblclick[0], a.dblclick[1]);
      if (a.type) await page.keyboard.type(a.type, { delay: 40 });
      if (a.press) await page.keyboard.press(a.press);
    }
    seen.acted = o.actions.length;
  }
  if (streaming && hold > 0) {
    // Watching: frames counted from here, for the hold.
    const before = await progress();
    await page.waitForTimeout(hold * 1000);
    const after = await progress();
    seen.held = {
      seconds: hold,
      videoFrames: after.vf - before.vf,
      socketFrames: after.ws - before.ws,
      socketAudioFrames: after.audio - before.audio,
      rtcAudioPackets: after.ra - before.ra,
    };
    pic = await picture(ev, await page.screenshot());
  }
  await page.screenshot({ path: o.out });
  const p = await progress();
  const transport = p.vf > 0 ? "webrtc" : p.ws > 0 ? "websockets" : null;
  seen = {
    ...seen,
    streaming,
    transport,
    candidates: p.r.pcs.map((x) => x.pair && `${x.pair.local?.type}/${x.pair.remote?.type}`).filter(Boolean),
    rtc: p.r.pcs,
    iceServers: p.r.ice,
    ...pic,
    address: mask(page.url()),
    tokenInAddress: /token=/.test(page.url()),
    sockets,
    socketAudioFrames: p.audio,
    errors: errors.slice(0, 20),
    consoleErrors,
    screenshot: o.out,
  };
} catch (e) {
  seen.error = mask(e.message).split("\n")[0];
  code = 1;
} finally {
  await browser.close();
}
console.log(JSON.stringify(seen));
if (expect === "stream" && !(seen.streaming && !seen.error)) code = 1;
if (expect === "none" && !(seen.page && seen.page < 500 && !seen.streaming && !seen.error)) code = 1;
if (expect === "refused" && !(seen.refused && !seen.streaming)) code = 1;
process.exit(code);
