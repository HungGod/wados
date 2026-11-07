// wadspaces.com: what WadSpaces is. No sign-in here. Streams have links of
// their own (https://<machine>.wadspaces.com/ws-<guid>/), served by the
// machine streaming them through its Cloudflare tunnel, not by this site.
import { motion } from "motion/react";
import {
  Bluetooth,
  Bot,
  Eye,
  FolderGit2,
  Gamepad2,
  Globe,
  Keyboard,
  Link2,
  ListChecks,
  MonitorSmartphone,
  Monitor,
  Network,
  Package,
  Radio,
  RotateCcw,
  ShieldCheck,
  Smartphone,
  Timer,
  Users,
  WandSparkles,
  type LucideIcon,
} from "lucide-react";
import { Logo, LogoMark } from "./components/Logo";

/** A screenshot of the machine (dark only, as WadSpaces is). */
function Shot({ name, alt }: { name: string; alt: string }) {
  return (
    <div className="overflow-hidden rounded-2xl shadow-deep ring-1 ring-line-strong">
      <img src={`/shots/${name}.webp`} alt={alt} className="block w-full" loading="lazy" width={1500} height={1000} />
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

function Card({ icon: Icon, title, children }: { icon: LucideIcon; title: string; children: React.ReactNode }) {
  return (
    <div className="rounded-3xl bg-surface p-6 ring-1 ring-line">
      <Icon className="size-5 text-accent" />
      <h3 className="mt-3 font-display text-lg font-semibold">{title}</h3>
      <p className="mt-2 text-sm text-muted">{children}</p>
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

/** What a wadspace is made of: the Builder's four tabs, in its colours. */
const PARTS = [
  {
    name: "Apps",
    color: "var(--part-app)",
    text: "VS Code, Krita, Obsidian, Steam, Dolphin and more, and WadBrowser for your web apps.",
  },
  {
    name: "Add-ons",
    color: "var(--part-ext)",
    text: "What extends an app: VS Code extensions, tools like Rust, Python or Node, and Claude Code.",
  },
  {
    name: "Secrets",
    color: "var(--part-key)",
    text: "Your sign-ins, kept as keys. Sign in to GitHub or Gmail once, and every wadspace with that key is signed in.",
  },
  {
    name: "Customize",
    color: "var(--part-look)",
    text: "Its name, tag, background and projects. Changing them never needs a rebuild.",
  },
];

/** Apps a wadspace can have (the Builder's catalog has more). */
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

/** What a public stream's share looks like, made up (nothing here is a real one). */
function StreamCard() {
  return (
    <div className="ws-glass rounded-3xl p-5 shadow-deep ring-1 ring-line-strong sm:p-6">
      <div className="flex items-center gap-2 text-sm text-muted">
        <span className="size-2 rounded-full bg-accent shadow-glow" />
        Public link · 1 watching
      </div>
      <div className="mt-4 space-y-2 font-mono text-sm">
        <div className="flex items-center gap-3 rounded-xl bg-surface-2 px-3 py-2.5 ring-1 ring-line">
          <Link2 className="size-4 shrink-0 text-muted" />
          <span className="truncate">https://studio.wadspaces.com/ws-……/</span>
        </div>
        <div className="flex items-center gap-3 rounded-xl bg-surface-2 px-3 py-2.5 ring-1 ring-line">
          <Users className="size-4 shrink-0 text-muted" />
          <span className="truncate">a user and a password</span>
        </div>
      </div>
      <p className="mt-4 text-xs text-faint">A new link each time, with a QR code. It ends when nobody's watching.</p>
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
          <a href="#agents" className="hidden rounded-lg px-3 py-2 text-muted hover:text-fg sm:block">
            Agents
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
              sign-ins. Run them on your own machines, switch between them in a keystroke, and stream one to any browser.
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
            <Shot name="launcher" alt="The launcher: wadspaces as cards in tabs, with the bar along the bottom" />
          </div>
        </section>

        <Section
          id="wadspaces"
          eyebrow="Wadspaces"
          title="A desktop for each thing you do."
          lead="A wadspace is a whole desktop with its own apps, add-ons, sign-ins and look. Design it in the Builder, and your machine builds it from ready-made packages, so a new one is ready in minutes."
        >
          <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
            {PARTS.map((p) => (
              <div key={p.name} className="rounded-3xl bg-surface p-6 ring-1 ring-line">
                <div className="flex items-center gap-2.5">
                  <span className="size-3 rounded-full" style={{ background: p.color }} />
                  <h3 className="font-display text-lg font-semibold">{p.name}</h3>
                </div>
                <p className="mt-3 text-sm text-muted">{p.text}</p>
              </div>
            ))}
          </div>
          <div className="mt-10 grid items-center gap-10 lg:grid-cols-[1fr_1.3fr]">
            <div className="space-y-6">
              <Point icon={WandSparkles} title="Design it in the Builder">
                Each wadspace is a stack: drag an app, a key or a whole section from one to another, and see its desktop before it's built.
              </Point>
              <Point icon={Package} title="Built from shared packages">
                Apps and tools come ready-made and are shared by every wadspace on the machine, so they download once.
              </Point>
              <Point icon={FolderGit2} title="Your projects come along">
                A GitHub repository is cloned in, a folder is opened in place, and a file is copied. Another of your machines' folders works too.
              </Point>
            </div>
            <Shot name="builder" alt="The Builder: two wadspaces as stacks of their customization, secrets, add-ons and apps, with the apps beside them" />
          </div>
          <ul className="mt-12 flex flex-wrap items-center gap-3" aria-label="Some of the apps a wadspace can have">
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
          lead="WadOS is the system underneath: installed from a USB stick onto an encrypted disk, and kept up to date as one image. Open several wadspaces at once, and move between them like apps."
        >
          <div className="grid items-center gap-10 lg:grid-cols-[1.3fr_1fr]">
            <Shot name="desktop" alt="The desktop: wadspaces and files as icons, a folder per tag, and the bar along the top" />
            <div className="space-y-6">
              <Point icon={Keyboard} title="Super+Tab and Alt+Space">
                Super+Tab switches between what's running. Alt+Space finds any wadspace or app by name.
              </Point>
              <Point icon={Monitor} title="Every screen a desktop">
                Scroll down from your wadspaces to the desktop. Each display has one of its own, and a wadspace can go full screen on any of them.
              </Point>
              <Point icon={Timer} title="Focus mode">
                Pick a wadspace and how long. Until the time's up, nothing else can come on screen.
              </Point>
              <Point icon={Globe} title="WadBrowser">
                Its own browser, with ads and trackers blocked. Any page can become an app with a window of its own.
              </Point>
            </div>
          </div>
          <div className="mt-14 grid gap-4 md:grid-cols-3">
            <Card icon={Network} title="Your machines, together">
              Machines on your tailnet find each other. Browse another's files, copy its wadspaces over, or stream one from it.
            </Card>
            <Card icon={RotateCcw} title="Updates you can undo">
              WadOS downloads an update in the background. Restart into it when you're ready, or go back to the one before.
            </Card>
            <Card icon={Bluetooth} title="Everything in the bar">
              Wi-Fi, Bluetooth, sound and brightness are a click away, with your notifications and what's running.
            </Card>
          </div>
        </Section>

        <Section
          id="agents"
          eyebrow="Agents"
          title="Agents with a wadspace of their own."
          lead="Give Claude Code a job in a wadspace. It works there on your project, not on your machine, and you watch what it builds as it goes."
        >
          <div className="grid gap-4 md:grid-cols-3">
            <Card icon={Bot} title="Runs where your project is">
              The agent works inside the wadspace, with its apps and keys. Your machine and your other wadspaces are out of its reach.
            </Card>
            <Card icon={Eye} title="A sandbox to try it in">
              When it has something to show, it opens a throwaway wadspace beside its checklist, so you can try the build before you trust it.
            </Card>
            <Card icon={ListChecks} title="Nothing ships by itself">
              It asks before it pushes or opens a pull request. The Agents app shows every job, what it's doing, and what's waiting for you.
            </Card>
          </div>
        </Section>

        <Section
          id="streams"
          eyebrow="Streams"
          title="Stream a wadspace to any browser."
          lead="Away from the machine? Stream a wadspace to a phone, a laptop or a friend's screen. Nothing to install, and no open port on the machine."
        >
          <div className="grid items-center gap-10 lg:grid-cols-2">
            <div className="space-y-6">
              <Point icon={ShieldCheck} title="Private, over your tailnet">
                Only your own devices can reach a private stream. It never leaves your network.
              </Point>
              <Point icon={Link2} title="Or a public link">
                At your machine's own address, through its tunnel. Open it in one tap, or sign in with the user and password that come with it.
              </Point>
              <Point icon={Gamepad2} title="Play together">
                Stream a game with up to four players, each with a link and a gamepad of their own.
              </Point>
              <Point icon={MonitorSmartphone} title="Any browser">
                Keyboard, mouse, touch and sound. Open it on the machine again and the stream ends, so the person there always comes first.
              </Point>
            </div>
            <StreamCard />
          </div>
          <div id="stream-link" className="mt-14 rounded-3xl bg-accent-soft p-6 ring-1 ring-line-strong sm:p-8">
            <h3 className="font-display text-xl font-semibold sm:text-2xl">Got a stream link?</h3>
            <p className="mt-2 max-w-3xl text-muted">
              Just open it. It looks like <span className="font-mono text-fg">https://&lt;machine&gt;.wadspaces.com/ws-……/</span>. A one-tap
              link signs you straight in; otherwise, sign in with the user and password you were sent. If it says the link has expired, the
              stream has ended: ask for a new one.
            </p>
          </div>
          <p className="mt-6 flex items-center gap-2 text-sm text-faint">
            <Smartphone className="size-4" /> An Android app for your machines and their streams is on its way.
          </p>
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
