//! Launch at login on macOS 13+, through `SMAppService.mainApp`.
//!
//! Nothing is cached: every call asks macOS, so the Settings switch matches
//! System Settings › General › Login Items › Open at Login, also after the
//! user removes LanShare there. macOS 11–12 have no SMAppService; the state
//! is then `Unsupported` and Settings hides the switch.
//!
//! A login launch is told apart from a normal one by the launch's Apple
//! event (`keyAELaunchedAsLogInItem`), which can only be read while AppKit is
//! handling that event: call [`launched_at_login`] first thing in `setup`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use objc2::MainThreadMarker;
use objc2::msg_send;
use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::{AnyClass, AnyObject};
use objc2_foundation::{NSAppleEventDescriptor, NSAppleEventManager, NSError};

// ServiceManagement ships with every macOS LanShare runs on (11+), but the
// SMAppService class only exists on 13+, so it is looked up by name at run
// time instead of being referenced as a symbol.
#[link(name = "ServiceManagement", kind = "framework")]
unsafe extern "C" {}

/// What the Settings switch shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginItem {
    /// macOS 11–12: no SMAppService. Hide the switch.
    Unsupported,
    /// Not running from /Applications or ~/Applications (a dev build, the
    /// DMG, a translocated copy in Downloads). Registering would point macOS
    /// at the wrong copy, so the switch is disabled with a note.
    NotInstalled,
    Off,
    On,
    /// Registered but not allowed to run (turned off in System Settings, or
    /// blocked by a profile): it will not open at login until allowed there.
    NeedsApproval,
}

impl LoginItem {
    pub fn as_str(self) -> &'static str {
        match self {
            LoginItem::Unsupported => "unsupported",
            LoginItem::NotInstalled => "notInstalled",
            LoginItem::Off => "off",
            LoginItem::On => "on",
            LoginItem::NeedsApproval => "needsApproval",
        }
    }
}

// SMAppServiceStatus, from <ServiceManagement/SMAppService.h>.
const STATUS_NOT_REGISTERED: isize = 0;
const STATUS_ENABLED: isize = 1;
const STATUS_REQUIRES_APPROVAL: isize = 2;
const STATUS_NOT_FOUND: isize = 3;

fn service_class() -> Option<&'static AnyClass> {
    AnyClass::get(c"SMAppService")
}

fn main_app_service() -> Option<Retained<AnyObject>> {
    let class = service_class()?;
    // SAFETY: `+[SMAppService mainAppService]` (macOS 13+) takes no arguments
    // and returns an object; the class exists (checked above).
    unsafe { msg_send![class, mainAppService] }
}

fn raw_status(service: &AnyObject) -> isize {
    // SAFETY: `-[SMAppService status]` returns an NSInteger enum.
    unsafe { msg_send![service, status] }
}

fn from_raw(raw: isize) -> LoginItem {
    match raw {
        STATUS_ENABLED => LoginItem::On,
        STATUS_REQUIRES_APPROVAL => LoginItem::NeedsApproval,
        // NOT_FOUND is what macOS reports after the user removes the app from
        // Open at Login; NOT_REGISTERED before the first register and after
        // an unregister.
        _ => LoginItem::Off,
    }
}

/// `/Applications/LanShare.app` when running from an app bundle.
fn bundle_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    // …/LanShare.app/Contents/MacOS/lanshare
    let bundle = exe.parent()?.parent()?.parent()?;
    (bundle.extension()? == "app").then(|| bundle.to_path_buf())
}

fn is_installed(bundle: &Path) -> bool {
    bundle.starts_with("/Applications")
        || std::env::var_os("HOME").is_some_and(|home| bundle.starts_with(Path::new(&home).join("Applications")))
}

fn installed() -> bool {
    bundle_path().is_some_and(|b| is_installed(&b))
}

/// The current state, read from macOS (one XPC round trip; fine to call on
/// every Settings load).
pub fn state() -> LoginItem {
    autoreleasepool(|_| {
        let Some(service) = main_app_service() else { return LoginItem::Unsupported };
        if !installed() {
            return LoginItem::NotInstalled;
        }
        from_raw(raw_status(&service))
    })
}

/// Turns launch at login on or off and returns the state macOS reports
/// afterwards, which is what the switch should show. Blocks on a call into
/// macOS, so run it with `spawn_blocking`. Turning it on makes macOS post
/// its own "Login Item Added" notification.
pub fn set(on: bool) -> Result<LoginItem, String> {
    autoreleasepool(|_| {
        let Some(service) = main_app_service() else { return Ok(LoginItem::Unsupported) };
        if !installed() {
            return Ok(LoginItem::NotInstalled);
        }
        let before = raw_status(&service);
        let result: Result<(), Retained<NSError>> = match (on, before) {
            (true, STATUS_ENABLED) | (false, STATUS_NOT_REGISTERED | STATUS_NOT_FOUND) => Ok(()),
            // SAFETY: BOOL-returning methods whose last argument is `NSError **`.
            (true, _) => unsafe { msg_send![&*service, registerAndReturnError: _] },
            (false, _) => unsafe { msg_send![&*service, unregisterAndReturnError: _] },
        };
        let after = from_raw(raw_status(&service));
        sync_relaunch_on_login(after == LoginItem::On);
        match result {
            Ok(()) => Ok(after),
            // "Already registered", "not registered", "denied by the user":
            // the state read back afterwards is the answer.
            Err(_) if on && after != LoginItem::Off => Ok(after),
            Err(_) if !on && after == LoginItem::Off => Ok(after),
            Err(e) => Err(describe_error(&e)),
        }
    })
}

/// Opens System Settings › General › Login Items (the way out of
/// `NeedsApproval`).
pub fn open_system_settings() {
    if let Some(class) = service_class() {
        // SAFETY: `+[SMAppService openSystemSettingsLoginItems]` takes no arguments.
        let _: () = unsafe { msg_send![class, openSystemSettingsLoginItems] };
    }
}

/// One line for the log, to debug launch at login on a real Mac.
pub fn diagnostics() -> String {
    autoreleasepool(|_| {
        let raw = main_app_service().map(|s| raw_status(&s));
        // No path: log lines never carry paths (core::logging).
        format!("SMAppService.mainApp status={raw:?} installed={}", installed())
    })
}

/// Only the error's domain and code: the description can quote the app's path.
fn describe_error(e: &NSError) -> String {
    format!("{} {}", e.domain(), e.code())
}

const fn four_cc(code: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*code)
}
const K_CORE_EVENT_CLASS: u32 = four_cc(b"aevt");
const K_AE_OPEN_APPLICATION: u32 = four_cc(b"oapp");
const KEY_AE_PROP_DATA: u32 = four_cc(b"prdt");
const KEY_AE_LAUNCHED_AS_LOG_IN_ITEM: u32 = four_cc(b"lgit");

/// The launch's Apple event: (event class, event ID, `keyAEPropData` enum
/// code). `None` off the main thread or when no Apple event is being handled.
fn current_event() -> Option<(u32, u32, Option<u32>)> {
    MainThreadMarker::new()?;
    autoreleasepool(|_| {
        let event = NSAppleEventManager::sharedAppleEventManager().currentAppleEvent()?;
        // SAFETY: plain getters on NSAppleEventDescriptor; AEEventClass,
        // AEEventID and AEKeyword are all FourCharCode (u32).
        let (class, id): (u32, u32) = unsafe { (msg_send![&*event, eventClass], msg_send![&*event, eventID]) };
        // SAFETY: as above; returns nil when the parameter is absent.
        let prop: Option<Retained<NSAppleEventDescriptor>> =
            unsafe { msg_send![&*event, paramDescriptorForKeyword: KEY_AE_PROP_DATA] };
        Some((class, id, prop.map(|p| p.enumCodeValue())))
    })
}

/// True when macOS opened LanShare as a login item.
///
/// Only meaningful on the main thread while AppKit is still handling the
/// launch's "open application" Apple event, i.e. inside
/// `applicationDidFinishLaunching:`. Tauri (tao) runs `Builder::setup`
/// synchronously from there, so call this at the top of `setup`.
pub fn launched_at_login() -> bool {
    current_event() == Some((K_CORE_EVENT_CLASS, K_AE_OPEN_APPLICATION, Some(KEY_AE_LAUNCHED_AS_LOG_IN_ITEM)))
}

/// The launch's Apple event as text, e.g. `aevt/oapp prdt=lgit`, for the log.
pub fn launch_event_text() -> String {
    let text = |c: u32| String::from_utf8_lossy(&c.to_be_bytes()).into_owned();
    match current_event() {
        None => "none".into(),
        Some((class, id, prop)) => format!("{}/{} prdt={}", text(class), text(id), prop.map_or("-".into(), text)),
    }
}

/// How soon after the GUI login a start still counts as "at login" for
/// [`opened_soon_after_login`].
const LOGIN_WINDOW_SECS: u64 = 120;

/// Fallback for a login launch whose Apple event carries no login-item flag:
/// launch at login is on, and the user's GUI session started moments ago. A
/// manual start that early would normally find LanShare already running.
/// Cheap: one utmpx scan, and an XPC call only in the first two minutes.
pub fn opened_soon_after_login() -> bool {
    seconds_since_console_login().is_some_and(|s| s < LOGIN_WINDOW_SECS) && state() == LoginItem::On
}

/// Seconds since this user's console (GUI) login, from utmpx (what `who` shows).
pub fn seconds_since_console_login() -> Option<u64> {
    let user = std::env::var("USER").ok()?;
    let field = |raw: &[libc::c_char]| -> Vec<u8> {
        // SAFETY: reinterpreting c_char as u8 over the same fixed-size array.
        let bytes = unsafe { std::slice::from_raw_parts(raw.as_ptr().cast::<u8>(), raw.len()) };
        bytes.iter().copied().take_while(|&b| b != 0).collect()
    };
    let mut latest: Option<libc::time_t> = None;
    // SAFETY: the utmpx iterator is only used here, on one thread, and every
    // entry is read before the next call.
    unsafe {
        libc::setutxent();
        loop {
            let entry = libc::getutxent();
            if entry.is_null() {
                break;
            }
            let entry = &*entry;
            if entry.ut_type == libc::USER_PROCESS
                && field(&entry.ut_line) == b"console"
                && field(&entry.ut_user) == user.as_bytes()
            {
                latest = latest.max(Some(entry.ut_tv.tv_sec));
            }
        }
        libc::endutxent();
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    now.checked_sub(u64::try_from(latest?).ok()?)
}

static RELAUNCH_DISABLED: AtomicBool = AtomicBool::new(false);

/// While launch at login is on, "Reopen windows when logging back in" must
/// not also bring LanShare back: that launch is not marked as a login item,
/// so the window would show. NSApplication counts these calls, so they are
/// kept paired. Call it at startup with the current state, and after `set`
/// (which already does).
pub fn sync_relaunch_on_login(login_item_on: bool) {
    if RELAUNCH_DISABLED.swap(login_item_on, Ordering::SeqCst) == login_item_on {
        return;
    }
    let Some(class) = AnyClass::get(c"NSApplication") else { return };
    // SAFETY: NSApp already exists (Tauri created it); Apple documents both
    // methods as thread safe.
    unsafe {
        let app: Retained<AnyObject> = msg_send![class, sharedApplication];
        if login_item_on {
            let _: () = msg_send![&*app, disableRelaunchOnLogin];
        } else {
            let _: () = msg_send![&*app, enableRelaunchOnLogin];
        }
    }
}
