// Logic for iphone-check.html. The page CSP allows only same-origin scripts,
// so everything lives here instead of inline handlers.
(function () {
  "use strict";
  const $ = (id) => document.getElementById(id);
  const MAX_LISTED = 200;

  // scripts/iphone_check.sh fills these in; the placeholders survive when it did not.
  const filled = (v) => (v && !v.startsWith("__") ? v : null);
  const here = location.origin;
  const isLocal = location.hostname.endsWith(".local");
  const isIp = /^\d+\.\d+\.\d+\.\d+$/.test(location.hostname);
  const localOrigin = filled(document.body.dataset.localOrigin) || (isLocal ? here : null);
  const ipOrigin = filled(document.body.dataset.ipOrigin) || (isIp ? here : null);

  // ---- current connection
  $("origin").textContent = here;
  $("host-kind").textContent = isLocal ? ".local 电脑名" : isIp ? "IP 地址" : "其他";
  $("secure").innerHTML = window.isSecureContext ? '<span class="ok">是</span>' : '<span class="bad">否</span>';
  // The iOS part of the user agent is frozen; Safari's own version tracks iOS.
  const safari = /Version\/([\d.]+)/.exec(navigator.userAgent);
  $("safari").textContent = safari ? safari[1] : navigator.userAgent;

  fetch("/api/ca", { cache: "no-store" })
    .then((r) => (r.ok ? r.json() : Promise.reject(new Error("HTTP " + r.status))))
    .then((d) => {
      $("ca-name").textContent = d.name;
      $("ca-expected").textContent = d.name;
    })
    .catch((e) => {
      $("ca-name").textContent = "读取失败（" + e.message + "）";
    });

  // ---- check 1: .local in Safari
  const link = (a, origin) => {
    if (origin) a.href = origin + location.pathname;
    else a.hidden = true;
  };
  link($("open-ip"), ipOrigin);
  link($("open-local"), localOrigin);
  if (localOrigin) $("local-host-label").textContent = new URL(localOrigin).hostname;
  const verdict = $("host-verdict");
  if (isLocal) {
    verdict.className = "ok";
    verdict.textContent = "你正在用 .local 地址访问。打开前没有出现证书警告 → 第 1 项通过。";
  } else if (isIp) {
    verdict.textContent =
      "你正在用 IP 地址访问。这里没有警告，说明证书已受信任；接着点「用 .local 打开」。如果 IP 地址也有警告，先在「证书信任设置」里打开 LanShare 证书。";
  }

  // ---- check 2: URLs for the Shortcut
  $("ca-local").textContent = localOrigin ? localOrigin + "/api/ca" : "（未知：请用 scripts/iphone_check.sh 启动）";
  $("ca-ip").textContent = ipOrigin ? ipOrigin + "/api/ca" : "（未知：请用 scripts/iphone_check.sh 启动）";

  async function copyText(text) {
    if (navigator.clipboard && navigator.clipboard.writeText && window.isSecureContext) {
      try {
        await navigator.clipboard.writeText(text);
        return true;
      } catch (_) {
        // fall through to the legacy path
      }
    }
    const ta = document.createElement("textarea");
    ta.value = text;
    ta.setAttribute("readonly", "");
    ta.style.position = "fixed";
    ta.style.opacity = "0";
    document.body.appendChild(ta);
    ta.select();
    ta.setSelectionRange(0, text.length);
    let ok = false;
    try {
      ok = document.execCommand("copy");
    } catch (_) {
      ok = false;
    }
    ta.remove();
    return ok;
  }

  document.querySelectorAll("[data-copy]").forEach((btn) => {
    btn.addEventListener("click", async () => {
      const text = $(btn.dataset.copy).textContent;
      const ok = await copyText(text);
      $("copy-status").textContent = ok ? "已复制：" + text : "复制失败，请长按地址手动复制";
    });
  });

  // ---- check 3: folder picker
  const input = $("dir-input");
  $("dir-support").textContent = "webkitdirectory" in input ? "是" : "否";

  const size = (n) => (n < 1024 ? n + " B" : n < 1048576 ? (n / 1024).toFixed(1) + " KB" : (n / 1048576).toFixed(1) + " MB");

  input.addEventListener("change", () => {
    const files = Array.from(input.files || []);
    const withPath = files.filter((f) => (f.webkitRelativePath || "").includes("/"));
    const tops = new Set(withPath.map((f) => f.webkitRelativePath.split("/")[0]));
    const depth = withPath.reduce((d, f) => Math.max(d, f.webkitRelativePath.split("/").length - 1), 0);

    const summary = $("dir-summary");
    if (files.length === 0) {
      summary.className = "bad";
      summary.textContent = "❌ 没有拿到任何文件（空文件夹也会这样，请选一个里面有文件的文件夹）";
    } else if (withPath.length === files.length) {
      summary.className = "ok";
      summary.textContent =
        "✅ " + files.length + " 个文件，全部带文件夹路径；顶层文件夹：" + Array.from(tops).join("、") + "；子文件夹层数：" + (depth - 1);
    } else {
      summary.className = "bad";
      summary.textContent = "❌ " + files.length + " 个文件，只有 " + withPath.length + " 个带文件夹路径";
    }

    const list = $("dir-files");
    list.replaceChildren();
    for (const f of files.slice(0, MAX_LISTED)) {
      const li = document.createElement("li");
      li.textContent = (f.webkitRelativePath || "（无路径）" + f.name) + " · " + size(f.size);
      list.appendChild(li);
    }
    if (files.length > MAX_LISTED) {
      const li = document.createElement("li");
      li.textContent = "…还有 " + (files.length - MAX_LISTED) + " 个";
      list.appendChild(li);
    }
    $("dir-result").hidden = false;
  });
})();
