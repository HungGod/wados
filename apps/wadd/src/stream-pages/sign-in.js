// The one-tap link (`/ws-<guid>/#token=<key>`). Browsers never send what's
// after the `#`, so the key reached this page and no server. It comes off
// the address bar (and history) at once, goes to the proxy in a POST once
// the page is on screen (a link preview made in the background doesn't
// open the stream), and the proxy sets the stream's cookie
// (`selkies_token`, HttpOnly); then the page loads again, with the cookie,
// and the proxy lets Selkies' client through. Without a key: the sign-in card.
(function () {
  var m = /(?:^#|&)token=([^&]+)/.exec(location.hash);
  if (!m) return;
  var key = m[1];
  try {
    key = decodeURIComponent(key);
  } catch (e) {}
  var here = location.pathname + location.search;
  history.replaceState(null, "", here);
  var root = document.documentElement;
  root.classList.add("opening");
  var sent = false;
  function show(state) {
    root.classList.remove("opening", "offline", "bad-link");
    root.classList.add(state);
  }
  function go() {
    if (sent || document.visibilityState !== "visible") return;
    sent = true;
    fetch(location.pathname, {
      method: "POST",
      headers: { "Content-Type": "application/x-www-form-urlencoded" },
      body: "token=" + encodeURIComponent(key),
      credentials: "same-origin",
      cache: "no-store",
    })
      .then(function (r) {
        if (r.status === 204) location.replace(here);
        else show("bad-link");
      })
      .catch(function () {
        sent = false;
        show("offline");
      });
  }
  document.addEventListener("visibilitychange", go);
  document.addEventListener("DOMContentLoaded", function () {
    var again = document.getElementById("again");
    if (again)
      again.addEventListener("click", function (e) {
        e.preventDefault();
        show("opening");
        go();
      });
  });
  go();
})();
