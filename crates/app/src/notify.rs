//! Notificaciones del sistema con `UNUserNotificationCenter`.
//!
//! Solo funcionan desde el `.app` (hace falta identificador de bundle). Con
//! `cargo run` la notificación se escribe en stderr. El permiso se pide la
//! primera vez que hay algo que notificar.

use std::cell::{Cell, RefCell};
use std::sync::{Arc, Mutex, OnceLock};

use block2::{DynBlock, RcBlock};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool, ProtocolObject};
use objc2::{AnyThread, define_class, sel};
use objc2_foundation::{
    NSArray, NSBundle, NSDictionary, NSError, NSObject, NSObjectProtocol, NSSet, NSString, NSUUID,
};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNMutableNotificationContent, UNNotificationAction,
    UNNotificationActionOptions, UNNotificationCategory, UNNotificationCategoryOptions,
    UNNotificationRequest, UNNotificationResponse, UNTextInputNotificationAction,
    UNTextInputNotificationResponse, UNUserNotificationCenter, UNUserNotificationCenterDelegate,
};

use crate::agent::alerts::{
    self, ACTION_ALLOW, ACTION_OPEN, ACTION_REPLY, Action, Alert, AlertKind, CATEGORY_INFO,
    CATEGORY_PERMISSION,
};
use crate::window::Waker;

thread_local! {
    static AUTHORIZATION_REQUESTED: Cell<bool> = const { Cell::new(false) };
}

/// Pide el permiso de notificaciones la primera vez que hay algo que mostrar.
fn request_authorization_once(center: &UNUserNotificationCenter) {
    if AUTHORIZATION_REQUESTED.with(|r| r.replace(true)) {
        return;
    }
    let handler = RcBlock::new(|granted: Bool, _error: *mut NSError| {
        if !granted.as_bool() {
            eprintln!("notificaciones no permitidas en Ajustes del Sistema");
        }
    });
    center.requestAuthorizationWithOptions_completionHandler(
        UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
        &handler,
    );
}

/// Muestra una notificación. Debe llamarse desde el hilo principal.
pub fn notify(title: &str, body: &str) {
    if NSBundle::mainBundle().bundleIdentifier().is_none() {
        eprintln!("[notificación] {title}: {body}");
        return;
    }
    let center = UNUserNotificationCenter::currentNotificationCenter();
    request_authorization_once(&center);
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(title));
    content.setBody(&NSString::from_str(body));
    let id = NSUUID::UUID().UUIDString();
    let request = UNNotificationRequest::requestWithIdentifier_content_trigger(&id, &content, None);
    center.addNotificationRequest_withCompletionHandler(&request, None);
}

// --- Notificaciones de agentes (v1.3) ---

/// Lo que el usuario hizo con una notificación de agente.
#[derive(Debug, Clone, PartialEq)]
pub struct Response {
    pub agent: String,
    /// La petición de permiso a la que se refería (no aplica a «terminado»).
    pub request_id: Option<u64>,
    pub action: Action,
}

static RESPONSES: Mutex<Vec<Response>> = Mutex::new(Vec::new());
static WAKER: OnceLock<Arc<Waker>> = OnceLock::new();

thread_local! {
    static DELEGATE: RefCell<Option<Retained<Delegate>>> = const { RefCell::new(None) };
}

/// Respuestas pendientes de procesar (hilo principal).
pub fn take_responses() -> Vec<Response> {
    std::mem::take(&mut *RESPONSES.lock().unwrap())
}

define_class!(
    // SAFETY: NSObject no impone requisitos a sus subclases y no tiene `Drop`.
    #[unsafe(super(NSObject))]
    #[name = "RTNotificationDelegate"]
    pub struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl UNUserNotificationCenterDelegate for Delegate {
        /// El usuario pulsó una acción o la notificación (puede ser otro hilo).
        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive(
            &self,
            _center: &UNUserNotificationCenter,
            response: &UNNotificationResponse,
            handler: &DynBlock<dyn Fn()>,
        ) {
            let action_id = response.actionIdentifier().to_string();
            let text = response
                .downcast_ref::<UNTextInputNotificationResponse>()
                .map(|r| r.userText().to_string());
            let info = response.notification().request().content().userInfo();
            let get = |key: &str| {
                info.objectForKey(&*NSString::from_str(key))
                    .and_then(|v| v.downcast::<NSString>().ok())
                    .map(|s| s.to_string())
            };
            if let (Some(agent), Some(action)) = (
                get("agent"),
                alerts::parse_action(&action_id, text.as_deref()),
            ) {
                RESPONSES.lock().unwrap().push(Response {
                    agent,
                    request_id: get("request").and_then(|r| r.parse().ok()),
                    action,
                });
                if let Some(waker) = WAKER.get() {
                    waker.on_main_thread(sel!(notificationResponses));
                }
            }
            handler.call(());
        }
    }
);

/// Registra el delegado y las categorías de acciones (una sola vez, hilo
/// principal). Sin bundle (`cargo run`) no hace nada.
pub fn install_agent_support(waker: Arc<Waker>) {
    if NSBundle::mainBundle().bundleIdentifier().is_none() || WAKER.set(waker).is_err() {
        return;
    }
    let delegate: Retained<Delegate> = {
        let this = Delegate::alloc().set_ivars(());
        // SAFETY: `init` de NSObject.
        unsafe { objc2::msg_send![super(this), init] }
    };
    let center = UNUserNotificationCenter::currentNotificationCenter();
    center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    DELEGATE.with(|d| *d.borrow_mut() = Some(delegate));

    let title = |s: &str| NSString::from_str(s);
    let allow = UNNotificationAction::actionWithIdentifier_title_options(
        &title(ACTION_ALLOW),
        &title("Permitir una vez"),
        UNNotificationActionOptions::empty(),
    );
    let reply = UNTextInputNotificationAction::actionWithIdentifier_title_options_textInputButtonTitle_textInputPlaceholder(
        &title(ACTION_REPLY),
        &title("Enviar respuesta"),
        UNNotificationActionOptions::empty(),
        &title("Denegar"),
        &title("Motivo para Claude"),
    );
    let open = UNNotificationAction::actionWithIdentifier_title_options(
        &title(ACTION_OPEN),
        &title("Abrir"),
        UNNotificationActionOptions::Foreground,
    );
    let permission =
        UNNotificationCategory::categoryWithIdentifier_actions_intentIdentifiers_options(
            &title(CATEGORY_PERMISSION),
            &NSArray::from_retained_slice(&[allow, reply.into_super(), open.clone()]),
            &NSArray::new(),
            UNNotificationCategoryOptions::empty(),
        );
    let info = UNNotificationCategory::categoryWithIdentifier_actions_intentIdentifiers_options(
        &title(CATEGORY_INFO),
        &NSArray::from_retained_slice(&[open]),
        &NSArray::new(),
        UNNotificationCategoryOptions::empty(),
    );
    center.setNotificationCategories(&NSSet::from_retained_slice(&[permission, info]));
}

/// Publica la notificación de un agente, con sus acciones.
pub fn notify_agent(alert: &Alert) {
    if NSBundle::mainBundle().bundleIdentifier().is_none() {
        eprintln!("[notificación] {}: {}", alert.title, alert.body);
        return;
    }
    let center = UNUserNotificationCenter::currentNotificationCenter();
    request_authorization_once(&center);
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(&alert.title));
    content.setBody(&NSString::from_str(&alert.body));
    let (category, request) = match alert.kind {
        AlertKind::Permission { request_id } => (CATEGORY_PERMISSION, Some(request_id)),
        AlertKind::Done | AlertKind::Error => (CATEGORY_INFO, None),
    };
    content.setCategoryIdentifier(&NSString::from_str(category));
    let agent = NSString::from_str(&alert.agent);
    let request_ns = request.map(|r| NSString::from_str(&r.to_string()));
    let (agent_key, request_key) = (NSString::from_str("agent"), NSString::from_str("request"));
    let info: Retained<NSDictionary<NSString, AnyObject>> = match &request_ns {
        Some(r) => NSDictionary::from_slices(&[&*agent_key, &*request_key], &[&*agent, &**r]),
        None => NSDictionary::from_slices(&[&*agent_key], &[&*agent]),
    };
    // SAFETY: el diccionario solo contiene cadenas.
    unsafe { content.setUserInfo(info.cast_unchecked()) };
    let id = NSUUID::UUID().UUIDString();
    let req = UNNotificationRequest::requestWithIdentifier_content_trigger(&id, &content, None);
    center.addNotificationRequest_withCompletionHandler(&req, None);
}
