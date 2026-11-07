# 0012: the phone

Phase 13. The phone remote is `../wadspaces-android`: the old app adapted, with its history, not a fresh one. The user
renamed `wadspaces-android-old` to it.

## What it is

The spike's phone (`src/surfaces/phone/`): the gates (Join your tailnet, a first machine, trust), then four tabs, Home,
Builder, Machines and Activity, under the WAD|SPACES app bar. It streams, builds, stops and edits wadspaces; it never
opens one on a machine's screen.

- **Kept from the old app:** the stream player (`StreamActivity`, `NativeVideo`, `native-video.js`), the theme (dark
  only now), the HTTP client, the `onTailnet` check, the build scripts, the debug fake mode.
- **Dropped:** Firebase (sign-in, Firestore, `wadspaces.properties`, the `FIREBASE_*` build fields), the client-side
  board (design B) and its goldens, v1's sets and layers, the wadspace password.

## Finding machines with no account

Android doesn't let an app read the Tailscale app's devices. The first time, the person types one machine's name
(MagicDNS) or tailnet address; it answers with my other machines (`GET /v1/remote/peers`), and the phone keeps the list.
Names are looked up each time, and only a tailnet address is ever called. Trust is Tailscale's WhoIs, as between
machines: the phone holds no token.

## wadd's remote API, for the phone

New routes, for my machines and trusted devices (`apps/wadd/src/remote.rs`), each the socket's own code:

- `peers`, `library`, `runs`, `builds`, `jobs`, `streams`, `streams/{id}/share` (its players' links too);
- `ops` and `undo/{id}`: the Builder's edits, as the machine's Builder sends them (`api::apply_op`, `apply_undo`);
- `notices`: the last 50, kept by the event bus as they're published (the shell keeps its own);
- `update`, `update/check`, and `restart`, which only restarts into a staged update, and only on a machine's own wadd.

`a_trusted_phone_does_what_the_app_needs` (tests/tailnet.rs) does what the app does, refused before trust and allowed
after; with `WADOS_PHONE_FIXTURES=<dir>` it writes the answers, which the app's core tests read. `the_phones_ops_read`
(wad-model) parses the app's edits as `Op`s.

## Testing

- **The app's core:** 13 tests against a real wadd's recorded answers (the client, the machine book, the ops, the
  spike's words and order).
- **The screens:** the debug build's fake mode on the user's phone (a moto g15, Android 15), screen by screen. The
  emulator crashed starting on this laptop.
- **Not yet:** the phone against a real machine. That needs a machine running this wadd (a new host image), the phone
  on the tailnet, and the trust ask answered on the machine's screen. Then a private and a public stream on the phone,
  and Play together's links.

## Not yet

- **Secrets on the phone** are read only: which key each app signs in with is chosen in the machine's Builder.
- **Projects** are added on the machine (its folders and repositories are there).
- **Edits need the machine online:** there's no account copy of an offline machine's wadspaces.
- **No push notifications:** the app reads its machines every 3 s while it's open.
- **Secure streams** aren't offered on the phone yet (the machine's are private-only and limited).
