// iOS Safari does not shrink the layout viewport when the keyboard opens.
// Pin the app to the *visual* viewport so the composer stays above the keyboard.

export function installViewportFix() {
  const vv = window.visualViewport;
  if (!vv) return;
  const root = document.documentElement;
  const update = () => {
    root.style.setProperty("--app-height", `${Math.round(vv.height)}px`);
    root.style.setProperty("--app-top", `${Math.round(vv.offsetTop)}px`);
    // A much shorter visual viewport means the on-screen keyboard is up.
    root.classList.toggle("keyboard-open", window.innerHeight - vv.height > 150);
  };
  vv.addEventListener("resize", update);
  vv.addEventListener("scroll", update);
  update();
}
