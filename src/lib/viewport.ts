// iOS Safari does not shrink the layout viewport when the keyboard opens.
// Pin the app to the *visual* viewport so the composer stays above the keyboard.

export function installViewportFix() {
  const vv = window.visualViewport;
  if (!vv) return;
  const root = document.documentElement;
  // The width and orientation at which the page was scrolled back down from above
  // its top (see below), until either changes (turning the phone round keeps the
  // width) or the scroll is seen to have held.
  let triedAt = "";
  const update = (settled = false) => {
    // A much shorter visual viewport means the on-screen keyboard is up. Compare it
    // with the layout viewport: on iOS window.innerHeight shrinks with the keyboard
    // too. Times scale, so pinch-zooming in does not count as a keyboard.
    const keyboard = root.clientHeight - vv.height * vv.scale > 150;
    const atRest = !keyboard && vv.scale === 1;
    // Without the keyboard or a zoom, the page cannot scroll past its content. In
    // the Home Screen app iOS breaks that when the phone turns back to portrait
    // (with viewport-fit=cover; a bare page does it too): the page is left scrolled
    // by the status bar's height, fixed layers are drawn that much lower than where
    // taps land, and it stays so until something else scrolls the page. Put the
    // scroll position back before measuring. Checked in the iOS 27 Simulator.
    const maxScroll = root.scrollHeight - root.clientHeight;
    if (atRest && window.scrollY > maxScroll + 1) window.scrollTo(0, maxScroll);
    // It happens above the top too, on a phone whose status bar moves from the top
    // edge to the side when it turns: the page is left scrolled to minus the status
    // bar's height. Every fixed layer, the file viewer as well, is then drawn that
    // much higher than where taps land, so --app-top cannot make up for it. Scroll
    // back only once things settle, as a scroll asked for while iOS is still
    // turning the page may not hold, and only once per width and orientation
    // unless it held, so that if iOS puts the position back the page does not
    // jump every half second. Seen on a real iPhone (iOS 27), where scrolling back
    // to the top once the turn had settled put taps right.
    if (settled && atRest) {
      const at = `${window.innerWidth} ${window.screen?.orientation?.type ?? ""}`;
      if (window.scrollY >= -1) triedAt = "";
      else if (triedAt !== at) {
        triedAt = at;
        window.scrollTo(0, 0);
      }
    }
    root.style.setProperty("--app-height", `${Math.round(vv.height)}px`);
    root.style.setProperty("--app-top", `${Math.round(vv.offsetTop)}px`);
    root.classList.toggle("keyboard-open", keyboard);
  };
  // iOS can report a change halfway through (the keyboard moving, the page coming
  // back from the background) and then stay quiet, which left the keyboard layout
  // on screen after the keyboard was gone. Measure again once things settle.
  let settle = 0;
  const onChange = () => {
    update();
    window.clearTimeout(settle);
    settle = window.setTimeout(() => update(true), 500);
  };
  vv.addEventListener("resize", onChange);
  vv.addEventListener("scroll", onChange);
  window.addEventListener("resize", onChange);
  window.addEventListener("pageshow", onChange);
  document.addEventListener("visibilitychange", onChange);
  // Turning the phone round may change no size at all.
  window.screen?.orientation?.addEventListener("change", onChange);
  // Settle after start-up too: the page may open already scrolled above its top.
  onChange();
}
