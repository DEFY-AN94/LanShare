// iOS Safari does not shrink the layout viewport when the keyboard opens.
// Pin the app to the *visual* viewport so the composer stays above the keyboard.

export function installViewportFix() {
  const vv = window.visualViewport;
  if (!vv) return;
  const root = document.documentElement;
  const update = () => {
    root.style.setProperty("--app-height", `${Math.round(vv.height)}px`);
    root.style.setProperty("--app-top", `${Math.round(vv.offsetTop)}px`);
    // A much shorter visual viewport means the on-screen keyboard is up. Compare it
    // with the layout viewport: on iOS window.innerHeight shrinks with the keyboard
    // too. Times scale, so pinch-zooming in does not count as a keyboard.
    root.classList.toggle("keyboard-open", root.clientHeight - vv.height * vv.scale > 150);
  };
  // iOS can report a change halfway through (the keyboard moving, the page coming
  // back from the background) and then stay quiet, which left the keyboard layout
  // on screen after the keyboard was gone. Measure again once things settle.
  let settle = 0;
  const onChange = () => {
    update();
    window.clearTimeout(settle);
    settle = window.setTimeout(update, 500);
  };
  vv.addEventListener("resize", onChange);
  vv.addEventListener("scroll", onChange);
  window.addEventListener("resize", onChange);
  window.addEventListener("pageshow", onChange);
  document.addEventListener("visibilitychange", onChange);
  update();
}
