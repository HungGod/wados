tailscaled's answers, shaped as Tailscale 1.104 marshals them: the LocalAPI's
GET /localapi/v0/status, /whois, /prefs (`ipnstate.Status`,
`apitype.WhoIsResponse`, `ipn.Prefs`; Go writes empty slices and maps as
`null`), a /watch-ipn-bus stream (one `ipn.Notify` per line), and what
`tailscale up --json` prints (tab-indented while it waits for the sign-in,
two-space-indented once it's running, other lines between). wad-tailscale's
tests read them, and its fake tailscaled (the `fake` feature) answers from
them. Made up: the keys, ids and addresses aren't anyone's.

Who's who in status-running.json, from the machine's side (wadspaces-surface,
owner@example.com, user 2001): desk (one of the owner's machines), Pixel 9
(the owner's phone), build-box (tagged: Tailscale's "tagged-devices" user),
friends-laptop (another person's, shared in). whois-tagged-owner.json is a
tagged node that still names the owner as its user (Headscale's way).
