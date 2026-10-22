//! macOS notifications with buttons (复制, 打开), through UNUserNotificationCenter.
//! tauri-plugin-notification uses the deprecated NSUserNotification API,
//! which cannot give LanShare what M8 needs (PLAN.md, M8 spike results); it
//! stays as the fallback when macOS does not let LanShare use this one.
//!
//! The delegate is set at the top of `setup`, which runs inside
//! applicationDidFinishLaunching, so a click that relaunched LanShare still
//! arrives; responses wait in a queue until the server is up.
//!
//! Notifications carry only the message id and the device id. The text a
//! button copies is read from the database when it is pressed, so a message
//! deleted since is not copied, and macOS keeps no more than the preview.

use std::ptr::NonNull;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Mutex, OnceLock, mpsc};
use std::time::Duration;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool, ProtocolObject};
use objc2::{AllocAnyThread, define_class, msg_send, sel};
use objc2_foundation::{NSArray, NSDictionary, NSError, NSObject, NSObjectProtocol, NSSet, NSString};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNAuthorizationStatus, UNMutableNotificationContent, UNNotification, UNNotificationAction,
    UNNotificationActionOptions, UNNotificationCategory, UNNotificationCategoryOptions, UNNotificationDefaultActionIdentifier,
    UNNotificationDismissActionIdentifier, UNNotificationInterruptionLevel, UNNotificationPresentationOptions, UNNotificationRequest,
    UNNotificationResponse, UNNotificationSettings, UNNotificationSound, UNUserNotificationCenter, UNUserNotificationCenterDelegate,
};

use crate::core::bridge::{MessageNotice, NoticeKind, Withdraw};
use crate::core::quick;

const CATEGORY_TEXT: &str = "lanshare.text";
const CATEGORY_FILE: &str = "lanshare.file";
const KEY_MESSAGE: &str = "message_id";
const KEY_DEVICE: &str = "device_id";

/// A button or a click on one of LanShare's notifications.
pub struct Response {
    pub action: &'static str,
    pub message_id: Option<i64>,
    pub device_id: Option<String>,
}

type Handler = Box<dyn Fn(Response) + Send + Sync>;
static HANDLER: OnceLock<Handler> = OnceLock::new();
static PENDING: Mutex<Vec<Response>> = Mutex::new(Vec::new());

/// What macOS said to the authorization request.
const ASKING: u8 = 0;
const ALLOWED: u8 = 1;
const DENIED: u8 = 2;
/// Not available to this build: it does not run from an app bundle.
const UNAVAILABLE: u8 = 3;
static STATUS: AtomicU8 = AtomicU8::new(ASKING);

fn deliver(r: Response) {
    match HANDLER.get() {
        Some(h) => h(r),
        None => PENDING.lock().unwrap_or_else(|p| p.into_inner()).push(r),
    }
}

/// Where responses go once the server is running; earlier ones are delivered now.
pub fn set_handler(f: impl Fn(Response) + Send + Sync + 'static) {
    if HANDLER.set(Box::new(f)).is_err() {
        return;
    }
    let queued = std::mem::take(&mut *PENDING.lock().unwrap_or_else(|p| p.into_inner()));
    if let Some(h) = HANDLER.get() {
        queued.into_iter().for_each(h);
    }
}

fn string_for(info: &NSDictionary, key: &str) -> Option<String> {
    let key = NSString::from_str(key);
    let value: Option<Retained<AnyObject>> = unsafe { msg_send![info, objectForKey: &*key] };
    value?.downcast::<NSString>().ok().map(|s| s.to_string())
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "LanShareNotificationDelegate"]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl UNUserNotificationCenterDelegate for Delegate {
        /// LanShare is in front but its window is not (it posts nothing while
        /// the window is in front): show the banner anyway.
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(
            &self,
            _center: &UNUserNotificationCenter,
            _notification: &UNNotification,
            completion: &block2::DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            completion.call((UNNotificationPresentationOptions::Banner
                | UNNotificationPresentationOptions::List
                | UNNotificationPresentationOptions::Sound,));
        }

        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive(
            &self,
            _center: &UNUserNotificationCenter,
            response: &UNNotificationResponse,
            completion: &block2::DynBlock<dyn Fn()>,
        ) {
            let id = response.actionIdentifier();
            let action = if *id == *unsafe { UNNotificationDefaultActionIdentifier } {
                quick::ACTION_SHOW
            } else if *id == *unsafe { UNNotificationDismissActionIdentifier } {
                quick::ACTION_DISMISS
            } else if id.to_string() == quick::ACTION_COPY {
                quick::ACTION_COPY
            } else if id.to_string() == quick::ACTION_OPEN {
                quick::ACTION_OPEN
            } else {
                quick::ACTION_DISMISS
            };
            let content = response.notification().request().content();
            let info = content.userInfo();
            let message_id = string_for(&info, KEY_MESSAGE).and_then(|s| s.parse().ok());
            let device_id = string_for(&info, KEY_DEVICE);
            deliver(Response { action, message_id, device_id });
            completion.call(());
        }
    }
);

impl Delegate {
    fn new() -> Retained<Self> {
        let this = Self::alloc().set_ivars(());
        unsafe { msg_send![super(this), init] }
    }
}

fn center() -> Retained<UNUserNotificationCenter> {
    UNUserNotificationCenter::currentNotificationCenter()
}

fn category(id: &str, action_id: &str, title: &str) -> Retained<UNNotificationCategory> {
    // No Foreground option: copying or opening does not bring LanShare forward.
    let action = UNNotificationAction::actionWithIdentifier_title_options(&NSString::from_str(action_id), &NSString::from_str(title), UNNotificationActionOptions::empty());
    UNNotificationCategory::categoryWithIdentifier_actions_intentIdentifiers_options(
        &NSString::from_str(id),
        &NSArray::from_retained_slice(&[action]),
        &NSArray::new(),
        UNNotificationCategoryOptions::empty(),
    )
}

/// UNUserNotificationCenter ends the process (an exception nothing can
/// catch) when it is not inside an app bundle: `tauri dev`, `cargo run`.
fn in_app_bundle() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.parent()?.parent()?.extension()? == "app"))
        .unwrap_or(false)
}

/// Sets the delegate and the buttons, and asks for permission (macOS asks
/// the user once). Call at the very top of `setup`.
pub fn install() {
    if !in_app_bundle() {
        // Builds outside LanShare.app use the plugin's plain notifications.
        STATUS.store(UNAVAILABLE, Ordering::SeqCst);
        tracing::info!("notifications: not running from an app bundle; using plain notifications");
        return;
    }
    let c = center();
    let delegate = Delegate::new();
    c.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    // The property is weak: the delegate lives as long as the process.
    std::mem::forget(delegate);
    c.setNotificationCategories(&NSSet::from_retained_slice(&[category(CATEGORY_TEXT, quick::ACTION_COPY, "复制"), category(CATEGORY_FILE, quick::ACTION_OPEN, "打开")]));
    let block = RcBlock::new(|granted: Bool, err: *mut NSError| {
        let code = unsafe { err.as_ref() }.map(|e| e.code());
        // Error 1 (UNErrorCodeNotificationsNotAllowed) comes with LanShare
        // turned off in System Settings › Notifications. Notifications still
        // go through this API, so they show once it is turned on again, and
        // Settings says where (`state()` reports "denied").
        let status = if granted.as_bool() { ALLOWED } else { DENIED };
        STATUS.store(status, Ordering::SeqCst);
        tracing::info!("notifications: granted={} error code={code:?}", granted.as_bool());
    });
    c.requestAuthorizationWithOptions_completionHandler(UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound | UNAuthorizationOptions::Badge, &block);
}

/// False when this API is not available to LanShare, so the caller falls
/// back to the plugin.
pub fn usable() -> bool {
    STATUS.load(Ordering::SeqCst) != UNAVAILABLE
}

fn user_info(message_id: i64, device_id: &str) -> Retained<NSDictionary<NSString, NSString>> {
    let (k1, k2) = (NSString::from_str(KEY_MESSAGE), NSString::from_str(KEY_DEVICE));
    let (v1, v2) = (NSString::from_str(&message_id.to_string()), NSString::from_str(device_id));
    NSDictionary::from_slices(&[&*k1, &*k2], &[&*v1, &*v2])
}

fn add(identifier: &str, content: &UNMutableNotificationContent) {
    let request = UNNotificationRequest::requestWithIdentifier_content_trigger(&NSString::from_str(identifier), content, None);
    let block = RcBlock::new(|err: *mut NSError| {
        if let Some(e) = unsafe { err.as_ref() } {
            tracing::warn!("a notification was not shown (error code {})", e.code());
        }
    });
    center().addNotificationRequest_withCompletionHandler(&request, Some(&block));
}

/// macOS 12 and later update a notification without a banner (interruption
/// levels); macOS 11 cannot, and `post` skips those updates.
pub fn quiet_updates() -> bool {
    static QUIET: OnceLock<bool> = OnceLock::new();
    *QUIET.get_or_init(|| UNMutableNotificationContent::new().respondsToSelector(sel!(setInterruptionLevel:)))
}

/// Shows a phone's message. Returns false if this API is not available.
pub fn post(n: &MessageNotice) -> bool {
    if !usable() {
        return false;
    }
    let c = UNMutableNotificationContent::new();
    c.setTitle(&NSString::from_str(&n.title));
    if let Some(sub) = &n.subtitle {
        c.setSubtitle(&NSString::from_str(sub));
    }
    c.setBody(&NSString::from_str(&n.body));
    c.setThreadIdentifier(&NSString::from_str(&n.device_id));
    c.setCategoryIdentifier(&NSString::from_str(match n.kind {
        NoticeKind::Text => CATEGORY_TEXT,
        NoticeKind::File => CATEGORY_FILE,
    }));
    let info = user_info(n.message_id, &n.device_id);
    // SAFETY: an NSDictionary of NSString keys and values is an NSDictionary of objects.
    let info: &NSDictionary = unsafe { &*(Retained::as_ptr(&info) as *const NSDictionary) };
    unsafe { c.setUserInfo(info) };
    if n.quiet {
        // Updates the list without a banner or sound. macOS 11 has no
        // interruption levels: it keeps the second notification as it is
        // (the notifier knows, see `quiet_updates`).
        if !quiet_updates() {
            return true;
        }
        c.setInterruptionLevel(UNNotificationInterruptionLevel::Passive);
    } else {
        c.setSound(Some(&UNNotificationSound::defaultSound()));
    }
    add(&n.identifier, &c);
    true
}

/// Shows a plain notification (a pairing request, an error). Returns false
/// if this API is not available.
pub fn post_plain(title: &str, body: &str) -> bool {
    if !usable() {
        return false;
    }
    let c = UNMutableNotificationContent::new();
    c.setTitle(&NSString::from_str(title));
    c.setBody(&NSString::from_str(body));
    c.setSound(Some(&UNNotificationSound::defaultSound()));
    add(&format!("plain.{}", crate::core::util::random_hex(8)), &c);
    true
}

pub fn withdraw(w: &Withdraw) {
    if !usable() {
        return;
    }
    let c = center();
    match w {
        Withdraw::All => c.removeAllDeliveredNotifications(),
        Withdraw::Ids(ids) => {
            let ids: Vec<Retained<NSString>> = ids.iter().map(|i| NSString::from_str(i)).collect();
            c.removeDeliveredNotificationsWithIdentifiers(&NSArray::from_retained_slice(&ids));
        }
        Withdraw::Device(device) => {
            let device = device.clone();
            let block = RcBlock::new(move |list: NonNull<NSArray<UNNotification>>| {
                let list = unsafe { list.as_ref() };
                let ids: Vec<Retained<NSString>> = list
                    .iter()
                    .filter(|n| n.request().content().threadIdentifier().to_string() == device)
                    .map(|n| n.request().identifier())
                    .collect();
                if !ids.is_empty() {
                    center().removeDeliveredNotificationsWithIdentifiers(&NSArray::from_retained_slice(&ids));
                }
            });
            c.getDeliveredNotificationsWithCompletionHandler(&block);
        }
    }
}

/// Whether the system shows LanShare's notifications. Blocking (a few ms).
pub fn state() -> &'static str {
    if !usable() {
        return "unavailable";
    }
    let (tx, rx) = mpsc::channel();
    let block = RcBlock::new(move |s: NonNull<UNNotificationSettings>| {
        let status = unsafe { s.as_ref() }.authorizationStatus();
        let _ = tx.send(match status {
            UNAuthorizationStatus::Authorized | UNAuthorizationStatus::Provisional | UNAuthorizationStatus::Ephemeral => "allowed",
            UNAuthorizationStatus::Denied => "denied",
            UNAuthorizationStatus::NotDetermined => "notDetermined",
            _ => "unknown",
        });
    });
    center().getNotificationSettingsWithCompletionHandler(&block);
    rx.recv_timeout(Duration::from_secs(3)).unwrap_or("unknown")
}
