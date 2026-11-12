// WadOS: a game's stream (gamepads on: a Dolphin or Steam wadspace's, Play
// together's) shows Selkies' touch gamepad straight away on a touch screen,
// a phone's: Selkies only opens it from its sidebar, which WadOS hides, so
// a phone had no controller. init-stream-display adds this to the page when
// the stream has gamepads. A mouse-and-keyboard screen is left alone.
(function () {
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
