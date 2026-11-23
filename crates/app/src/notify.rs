//! Notificaciones del sistema con `UNUserNotificationCenter`.
//!
//! Solo funcionan desde el `.app` (hace falta identificador de bundle). Con
//! `cargo run` la notificación se escribe en stderr. El permiso se pide la
//! primera vez que hay algo que notificar.

use std::cell::Cell;

use block2::RcBlock;
use objc2::runtime::Bool;
use objc2_foundation::{NSBundle, NSError, NSString, NSUUID};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNMutableNotificationContent, UNNotificationRequest,
    UNUserNotificationCenter,
};

thread_local! {
    static AUTHORIZATION_REQUESTED: Cell<bool> = const { Cell::new(false) };
}

/// Muestra una notificación. Debe llamarse desde el hilo principal.
pub fn notify(title: &str, body: &str) {
    if NSBundle::mainBundle().bundleIdentifier().is_none() {
        eprintln!("[notificación] {title}: {body}");
        return;
    }
    let center = UNUserNotificationCenter::currentNotificationCenter();
    if !AUTHORIZATION_REQUESTED.with(|r| r.replace(true)) {
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
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(title));
    content.setBody(&NSString::from_str(body));
    let id = NSUUID::UUID().UUIDString();
    let request = UNNotificationRequest::requestWithIdentifier_content_trigger(&id, &content, None);
    center.addNotificationRequest_withCompletionHandler(&request, None);
}
