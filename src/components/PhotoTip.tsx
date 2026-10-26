// One-time note before the first upload on iPhone/iPad: picking from the
// photo library makes iOS convert HEIC/HEVC/PNG; originals need the Files route.
// Verified on a real iPhone (iOS 27): no <input accept> value avoids the conversion.

import { useRef, useState, type ReactNode } from "react";
import { useBackdropClose } from "../lib/useBackdropClose";

const KEY = "lanshare.photoTipSeen";

function isAppleTouchDevice(): boolean {
  const ua = navigator.userAgent;
  return /iPhone|iPad|iPod/.test(ua) || (/Macintosh/.test(ua) && navigator.maxTouchPoints > 1);
}

function seen(): boolean {
  try {
    return window.localStorage.getItem(KEY) === "1";
  } catch {
    return false;
  }
}

function markSeen() {
  try {
    window.localStorage.setItem(KEY, "1");
  } catch {
    // private mode: show it again next time, harmless
  }
}

/**
 * Wraps "open the file picker": shows the tip first when needed. `pick` must
 * open the <input type=file>; it runs inside the tip's button tap, so iOS
 * still treats it as a user gesture.
 */
export function usePhotoTip(pick: () => void): { open: () => void; element: ReactNode } {
  const [visible, setVisible] = useState(false);
  const [dontShow, setDontShow] = useState(true);
  const pickRef = useRef(pick);
  const backdrop = useBackdropClose(() => setVisible(false));
  pickRef.current = pick;

  const open = () => {
    if (isAppleTouchDevice() && !seen()) setVisible(true);
    else pickRef.current();
  };

  const proceed = () => {
    if (dontShow) markSeen();
    setVisible(false);
    pickRef.current();
  };

  const element = visible ? (
    <div className="modal-backdrop" {...backdrop}>
      <div className="modal dialog photo-tip" role="dialog" aria-modal="true">
        <h2 className="dialog-title">从 iPhone 上传照片和视频</h2>
        <ul className="tip-list">
          <li>
            从<b>「照片图库」</b>选最快，但 iOS 会自动转换格式：HEIC 照片变 JPEG，HEVC 视频变 H.264，截图 PNG 变
            JPEG。分辨率不变，视频文件可能会变大。
          </li>
          <li>
            需要<b>原片</b>（原始格式和画质）时：先在「照片」App 里选中，点分享 › <b>保存到“文件”</b>，再回到这里选
            <b>「选取文件」</b>。
          </li>
          <li>ProRAW（DNG）照片和「文件」里的其他文件不受影响。</li>
        </ul>
        <label className="tip-check">
          <input type="checkbox" checked={dontShow} onChange={(e) => setDontShow(e.target.checked)} />
          <span>不再提示</span>
        </label>
        <div className="dialog-actions">
          <button className="btn" onClick={() => setVisible(false)}>
            取消
          </button>
          <button className="btn btn-primary" onClick={proceed}>
            继续选择
          </button>
        </div>
      </div>
    </div>
  ) : null;

  return { open, element };
}
