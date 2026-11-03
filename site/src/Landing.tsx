// wadspaces.com: what WadSpaces is. No sign-in here; streams have links of
// their own (https://ws-<id>.wadspaces.com, served by the machine streaming
// them through its Cloudflare tunnel, not by this site).
import { motion } from "motion/react";
import {
  Bluetooth,
  FolderGit2,
  Globe,
  HardDrive,
  KeyRound,
  Keyboard,
  Layers,
  Link2,
  MonitorSmartphone,
  NotebookPen,
  Radio,
  Timer,
  Usb,
  WandSparkles,
  type LucideIcon,
} from "lucide-react";
import { Logo, LogoMark } from "./components/Logo";

/** A screenshot of the machine (dark only, as WadSpaces is). */
function Shot({ name, alt }: { name: string; alt: string }) {
  return (
    <div className="overflow-hidden rounded-2xl shadow-deep ring-1 ring-line-strong">
      <img src={`/shots/${name}.webp`} alt={alt} className="block w-full" loading="lazy" width={1280} height={800} />
    </div>
  );
}

function Point({ icon: Icon, title, children }: { icon: LucideIcon; title: string; children: React.ReactNode }) {
  return (
    <div className="flex gap-3">
      <span className="mt-0.5 grid size-9 shrink-0 place-items-center rounded-xl bg-surface-2 text-accent ring-1 ring-line">
        <Icon className="size-4" />
      </span>
      <div>
        <h4 className="font-display font-semibold">{title}</h4>
        <p className="mt-0.5 text-sm text-muted">{children}</p>
      </div>
    </div>
  );
}

function Section({ id, eyebrow, title, lead, children }: { id: string; eyebrow: string; title: string; lead: string; children: React.ReactNode }) {
  return (
    <section id={id} aria-labelledby={`${id}-title`} className="mx-auto w-full max-w-6xl px-4 py-16 sm:px-6 sm:py-24">
      <p className="text-xs font-semibold uppercase tracking-[0.2em] text-accent">{eyebrow}</p>
      <h2 id={`${id}-title`} className="mt-3 max-w-3xl font-display text-3xl font-semibold tracking-tight sm:text-4xl">
        {title}
      </h2>
      <p className="mt-4 max-w-3xl text-base text-muted sm:text-lg">{lead}</p>
      <div className="mt-10">{children}</div>
    </section>
  );
}

/** The three layers a wadspace is made of, in the Builder's colours. */
const LAYERS = [
  {
    name: "Template",
    color: "var(--layer-template)",
    text: "Its apps: VS Code, Krita, Obsidian, Steam, a browser for each web app. One template can build on another.",
  },
  {
    name: "Extensions",
    color: "var(--layer-extensions)",
    text: "What it adds: tools, VS Code extensions, web apps, and what opens when it starts.",
  },
  {
    name: "Customization",
    color: "var(--layer-customization)",
    text: "What makes it yours: its name, background, projects and keys. It's handed over when it starts, so it never needs building.",
  },
];

/** Apps a template can have (the Builder's catalog has more). */
const APPS = [
  ["vscode", "VS Code"],
  ["obsidian", "Obsidian"],
  ["krita", "Krita"],
  ["audacity", "Audacity"],
  ["obs", "OBS Studio"],
  ["openshot", "OpenShot"],
  ["lmms", "LMMS"],
  ["libresprite", "LibreSprite"],
  ["tiled", "Tiled"],
  ["steam", "Steam"],
  ["dolphin", "Dolphin"],
];

/** What a stream's link looks like, made up (nothing here is a real one). */
function StreamCard() {
  return (
    <div className="ws-glass rounded-3xl p-5 shadow-deep ring-1 ring-line-strong sm:p-6">
      <div className="flex items-center gap-2 text-sm text-muted">
        <span className="size-2 rounded-full bg-accent shadow-glow" />
        Streaming: 1 watching
      </div>
      <div className="mt-4 space-y-2 font-mono text-sm">
        <div className="flex items-center gap-3 rounded-xl bg-surface-2 px-3 py-2.5 ring-1 ring-line">
          <Link2 className="size-4 shrink-0 text-muted" />
          <span className="truncate">https://ws-…….wadspaces.com</span>
        </div>
        <div className="flex items-center gap-3 rounded-xl bg-surface-2 px-3 py-2.5 ring-1 ring-line">
          <KeyRound className="size-4 shrink-0 text-muted" />
          <span className="truncate">a username and a password</span>
        </div>
      </div>
      <p className="mt-4 text-xs text-faint">A new link and login each time. It ends when nobody's watching.</p>
    </div>
  );
}

export default function Landing() {
  return (
    <div className="ws-backdrop relative min-h-screen overflow-x-clip">
      <div aria-hidden className="pointer-events-none absolute inset-x-0 top-0 hidden h-[900px] overflow-hidden dark:block">
        <motion.div
          className="absolute -left-40 top-24 size-[520px] rounded-full bg-[#ff3d81]/20 blur-[120px]"
          animate={{ x: [0, 60, 0], y: [0, -40, 0] }}
          transition={{ duration: 18, repeat: Infinity, ease: "easeInOut" }}
        />
        <motion.div
          className="absolute -right-32 top-80 size-[460px] rounded-full bg-[#c6ff1f]/10 blur-[120px]"
          animate={{ x: [0, -50, 0], y: [0, 30, 0] }}
          transition={{ duration: 22, repeat: Infinity, ease: "easeInOut" }}
        />
      </div>

      <header className="relative mx-auto flex w-full max-w-6xl items-center justify-between px-4 py-5 sm:px-6">
        <a href="/" className="flex items-center gap-2.5" aria-label="WadSpaces">
          <LogoMark className="size-9" />
          <span className="font-display text-lg font-semibold tracking-tight">WadSpaces</span>
        </a>
        <nav className="flex items-center gap-1 text-sm">
          <a href="#wadspaces" className="hidden rounded-lg px-3 py-2 text-muted hover:text-fg sm:block">
            Wadspaces
          </a>
          <a href="#machine" className="hidden rounded-lg px-3 py-2 text-muted hover:text-fg sm:block">
            The machine
          </a>
          <a href="#streams" className="hidden rounded-lg px-3 py-2 text-muted hover:text-fg sm:block">
            Streams
          </a>
        </nav>
      </header>

      <main className="relative">
        <section className="mx-auto w-full max-w-6xl px-4 pb-16 pt-10 text-center sm:px-6 sm:pt-16">
          <motion.div initial={{ opacity: 0, y: 20 }} animate={{ opacity: 1, y: 0 }} transition={{ type: "spring", stiffness: 200, damping: 26 }}>
            <Logo className="mx-auto h-28 sm:h-36" />
            <h1 className="mx-auto mt-8 max-w-3xl font-display text-4xl font-semibold tracking-tight sm:text-6xl">
              Your workspaces, on your machines.
            </h1>
            <p className="mx-auto mt-5 max-w-2xl text-base text-muted sm:text-lg">
              WadSpaces is a computer that boots into desktops you design: a wadspace for each thing you do, with its own apps, files and
              sign-ins. Build them from layers, run them on your own hardware, and stream one to any browser when you're away from it.
            </p>
            <div className="mt-8 flex flex-wrap items-center justify-center gap-3">
              <a href="#wadspaces" className="inline-flex h-11 items-center gap-2 rounded-xl bg-accent px-5 text-sm font-semibold text-accent-fg shadow-glow">
                See how it works
              </a>
              <a href="#stream-link" className="inline-flex h-11 items-center gap-2 rounded-xl bg-surface-2 px-5 text-sm font-semibold ring-1 ring-line hover:ring-line-strong">
                <Radio className="size-4" /> Got a stream link?
              </a>
            </div>
          </motion.div>
          <div className="mx-auto mt-14 max-w-5xl">
            <Shot name="launcher" alt="The launcher: your wadspaces as cards, what's running first, the bar along the bottom" />
          </div>
        </section>

        <Section
          id="wadspaces"
          eyebrow="Wadspaces"
          title="A desktop for each thing you do."
          lead="A wadspace is a whole desktop made of layers. Shared layers are built once and reused, so a new wadspace on an old template is ready in seconds."
        >
          <div className="grid gap-4 md:grid-cols-3">
            {LAYERS.map((l) => (
              <div key={l.name} className="rounded-3xl bg-surface p-6 ring-1 ring-line">
                <div className="flex items-center gap-2.5">
                  <span className="size-3 rounded-full" style={{ background: l.color }} />
                  <h3 className="font-display text-lg font-semibold">{l.name}</h3>
                </div>
                <p className="mt-3 text-sm text-muted">{l.text}</p>
              </div>
            ))}
          </div>
          <div className="mt-10 grid items-center gap-10 lg:grid-cols-[1fr_1.3fr]">
            <div className="space-y-6">
              <Point icon={WandSparkles} title="Design it in the Builder">
                Stack a template, Extensions and a Customization, and see the desktop you'll get before it's built.
              </Point>
              <Point icon={Layers} title="Built on the machine">
                Each layer is its own image, built on the machine where it runs, and shared by every wadspace on it.
              </Point>
              <Point icon={KeyRound} title="Sign in once">
                Saved sign-ins (GitHub, VS Code, your web apps) are shared by every wadspace that uses them.
              </Point>
            </div>
            <Shot name="builder" alt="The Builder: each wadspace a stack of its customization, secrets, add-ons and apps" />
          </div>
          <ul className="mt-12 flex flex-wrap items-center gap-3" aria-label="Some of the apps a template can have">
            {APPS.map(([id, name]) => (
              <li key={id} className="flex items-center gap-2 rounded-full bg-surface py-1.5 pl-1.5 pr-3.5 text-sm ring-1 ring-line">
                <img src={`/apps/${id}/icon.png`} alt="" className="size-6 rounded-md" loading="lazy" />
                {name}
              </li>
            ))}
            <li className="flex items-center gap-2 rounded-full bg-surface py-1.5 pl-1.5 pr-3.5 text-sm ring-1 ring-line">
              <img src="/brand/wadbrowser-icon-dark.svg" alt="" className="size-6 rounded-md" loading="lazy" />
              WadBrowser
            </li>
          </ul>
        </Section>

        <Section
          id="machine"
          eyebrow="The machine"
          title="It boots straight into your wadspaces."
          lead="A WadSpaces machine is a laptop or desktop running the WadSpaces OS. Open several wadspaces at once and switch between them like apps."
        >
          <div className="grid items-center gap-10 lg:grid-cols-[1.3fr_1fr]">
            <Shot name="desktop" alt="The desktop: wadspaces as glass tiles, a folder per tag, and the bar along the top" />
            <div className="space-y-6">
              <Point icon={Keyboard} title="Super+Tab between them">
                Every wadspace runs full screen. Super+Tab and Super+1…9 switch, and the bar above has Wi-Fi, sound and power.
              </Point>
              <Point icon={Timer} title="Focus mode">
                Pick what you're working on and for how long. Until the time's up, only those can come on screen.
              </Point>
              <Point icon={NotebookPen} title="WadNotes">
                Notes, tasks and reminders for each wadspace, and a calendar they all share.
              </Point>
              <Point icon={Globe} title="WadBrowser">
                Its own browser, where each web app gets a window of its own.
              </Point>
            </div>
          </div>
          <div className="mt-14 grid gap-4 md:grid-cols-3">
            <div className="rounded-3xl bg-surface p-6 ring-1 ring-line">
              <FolderGit2 className="size-5 text-accent" />
              <h3 className="mt-3 font-display text-lg font-semibold">Your projects come along</h3>
              <p className="mt-2 text-sm text-muted">
                GitHub repositories are cloned into each wadspace that uses them. Folders on the machine can be copied in, or mounted.
              </p>
            </div>
            <div className="rounded-3xl bg-surface p-6 ring-1 ring-line">
              <Usb className="size-5 text-accent" />
              <h3 className="mt-3 font-display text-lg font-semibold">Devices go where they're needed</h3>
              <p className="mt-2 text-sm text-muted">
                Plug in a gamepad, webcam or security key and hand it to a wadspace. It remembers next time.
              </p>
            </div>
            <div className="rounded-3xl bg-surface p-6 ring-1 ring-line">
              <div className="flex gap-2 text-accent">
                <HardDrive className="size-5" />
                <Bluetooth className="size-5" />
              </div>
              <h3 className="mt-3 font-display text-lg font-semibold">Drives and Bluetooth</h3>
              <p className="mt-2 text-sm text-muted">
                USB drives show up in a wadspace's Drives folder and eject safely. Headphones and pads pair from the bar.
              </p>
            </div>
          </div>
        </Section>

        <Section
          id="streams"
          eyebrow="Streams"
          title="Stream a wadspace to any browser."
          lead="Away from the machine? Stream a wadspace to a phone, a laptop or a friend's screen. The machine needs no open port: streams go out through a secure tunnel."
        >
          <div className="grid items-center gap-10 lg:grid-cols-2">
            <div className="space-y-6">
              <Point icon={Radio} title="One button">
                Stream it from the launcher or from Home. The wadspace moves off the machine's screen and onto the stream.
              </Point>
              <Point icon={Link2} title="A private link and its own login">
                Every stream gets a new link and a username and password to hand on with it. Nobody gets in without both.
              </Point>
              <Point icon={MonitorSmartphone} title="Any browser">
                Nothing to install: open the link, sign in, and the desktop is there, keyboard, mouse, touch and sound.
              </Point>
            </div>
            <StreamCard />
          </div>
          <div id="stream-link" className="mt-14 rounded-3xl bg-accent-soft p-6 ring-1 ring-line-strong sm:p-8">
            <h3 className="font-display text-xl font-semibold sm:text-2xl">Got a stream link?</h3>
            <p className="mt-2 max-w-3xl text-muted">
              Just open it: it looks like <span className="font-mono text-fg">https://ws-…….wadspaces.com</span>. Your browser asks for
              the username and password you were sent with it. If the link says it's not found, that stream has ended: ask for a new one.
            </p>
          </div>
        </Section>
      </main>

      <footer className="relative border-t border-line">
        <div className="mx-auto flex w-full max-w-6xl flex-col items-center justify-between gap-3 px-4 py-8 text-sm text-faint sm:flex-row sm:px-6">
          <div className="flex items-center gap-2">
            <LogoMark className="size-6" />
            <span>WadSpaces · your workspaces, on your machines</span>
          </div>
          <span>© 2026 WadSpaces</span>
        </div>
      </footer>
    </div>
  );
}
