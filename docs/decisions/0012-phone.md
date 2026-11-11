# 0012: the phone

Phase 13. The phone remote is `../wadspaces-android`: the old app adapted, with its history, not a fresh one. The user
renamed `wadspaces-android-old` to it.

## What it is

The spike's phone (`src/surfaces/phone/`): the gates (signing in, Join your tailnet, trust), then four tabs, Home,
Builder, Machines and Activity, under the WAD|SPACES app bar. It streams, builds, stops and edits wadspaces; it never
opens one on a machine's screen.

- **Kept from the old app:** the stream player (`StreamActivity`, `NativeVideo`, `native-video.js`), the theme (dark
  only now), the HTTP client, the `onTailnet` check, the build scripts, the debug fake mode.
- **Dropped:** Firebase (sign-in, Firestore, `wadspaces.properties`, the `FIREBASE_*` build fields), the client-side
  board (design B) and its goldens, v1's sets and layers, the wadspace password. Firebase came back later: the
  account's sign-in, its machines and secret files (0013).

## Finding machines: the account only

Android doesn't let an app read the Tailscale app's devices. At first the person typed one machine's name (MagicDNS) or
tailnet address, and a machine that trusted the phone named the others (`GET /v1/remote/peers`), with Tailscale's WhoIs
as the trust. That's gone: the user decided the account (0013) is the only way the phone knows a machine. Signed in,
it reads the account's machines, and its ID token is its trust; signed out, it knows none and opens on the sign-in
page. Names are looked up each time, and only a tailnet address is ever called.

One of the account's machines whose wadd doesn't take the token (signed out there, or signed in to another account)
can still be asked to trust the phone on its own screen: that's wadd's own trust ask, which it keeps for other devices.

## wadd's remote API, for the phone

New routes, for my machines and trusted devices (`apps/wadd/src/remote.rs`), each the socket's own code:

- `peers` (the phone no longer reads it: the account lists its machines), `library`, `runs`, `builds`, `jobs`, `streams`, `streams/{id}/share` (its players' links too);
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
