// WadOS: a game's stream (gamepads on: a Dolphin or Steam wadspace's, Play
// together's). init-stream-display adds this to the page when the stream
// has gamepads; it runs before Selkies' own scripts (theirs are modules).
(function () {
  // Play together's other players are Selkies' viewers: the server takes
  // only their own gamepad from them (its slot), and nothing else they send.
  // Told so, Selkies' dashboard shows a viewer nothing but a gamepad button:
  // no menu, no fullscreen. So it's told it's at the controls; the server
  // still decides what each player's input may do. (Only a game's stream
  // has viewers in WadOS. A capturing listener: ahead of the dashboard's.)
  // Its picture is player 1's screen's size, not its own: kept whole, in
  // proportion (Selkies' page stretches it over the window, as it would a
  // screen sized to fit it).
  var viewer = function () {
    var root = document.documentElement;
    if (root.classList.contains('wados-player')) return;
    root.classList.add('wados-player');
    var style = document.createElement('style');
    style.textContent = 'html.wados-player #stream, html.wados-player #videoCanvas { object-fit: contain !important; }';
    (document.head || root).appendChild(style);
  };
  window.addEventListener('message', function (e) {
    var d = e.data;
    if (e.source !== window || !d || d.type !== 'clientRoleUpdate' || d.role !== 'viewer') return;
    e.stopImmediatePropagation();
    viewer();
    window.postMessage({ type: 'clientRoleUpdate', role: 'controller' }, window.location.origin);
  }, true);

  // On a touch screen (a phone's), Selkies' touch gamepad, straight away:
  // Selkies only opens it from its sidebar, which WadOS hides, so a phone
  // had no controller. A mouse-and-keyboard screen is left alone.
  var touch = (window.matchMedia && matchMedia('(pointer: coarse)').matches) || navigator.maxTouchPoints > 0;
  if (!touch) return;
  // Its controls, not its host (that's on the page, empty, while it's off).
  var shown = function () { return !!document.getElementById('universal-touch-gamepad-controls-overlay'); };
  var asked = 0;
  // Selkies listens once its page has drawn: asked again until it's there
  // (a toggle at a time, each given time to show, so it's never shut again).
  var t = setInterval(function () {
    if (shown() || asked >= 20) {
      clearInterval(t);
      return;
    }
    // Its video: a private stream's (#videoStream, #videoCanvas), or a
    // public one's, over WebRTC (#stream).
    if (!document.querySelector('#videoStream, #videoCanvas, #stream')) return;
    asked++;
    window.postMessage({ type: 'toggleTouchGamepad' }, window.location.origin);
  }, 1500);
})();
