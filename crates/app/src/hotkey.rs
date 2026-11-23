//! Atajo global ⌃⌥N («Nuevo agente…» desde cualquier app).
//!
//! `RegisterEventHotKey` (Carbon) es la forma de registrar un atajo global
//! que **no** pide el permiso de Accesibilidad. Los eventos llegan por el
//! run loop principal.

use std::ffi::c_void;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, OnceLock};

use objc2::sel;

use crate::window::Waker;

type OsStatus = i32;
type Ref = *mut c_void;

#[repr(C)]
struct EventTypeSpec {
    event_class: u32,
    event_kind: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct EventHotKeyId {
    signature: u32,
    id: u32,
}

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn GetApplicationEventTarget() -> Ref;
    fn InstallEventHandler(
        target: Ref,
        handler: extern "C" fn(Ref, Ref, *mut c_void) -> OsStatus,
        count: u32,
        types: *const EventTypeSpec,
        user_data: *mut c_void,
        out_ref: *mut Ref,
    ) -> OsStatus;
    fn RegisterEventHotKey(
        key_code: u32,
        modifiers: u32,
        id: EventHotKeyId,
        target: Ref,
        options: u32,
        out_ref: *mut Ref,
    ) -> OsStatus;
}

const CLASS_KEYBOARD: u32 = u32::from_be_bytes(*b"keyb");
const KIND_HOTKEY_PRESSED: u32 = 5;
const CONTROL_KEY: u32 = 0x1000;
const OPTION_KEY: u32 = 0x0800;
/// Código de tecla (ANSI) de la «N».
const KEY_N: u32 = 45;
const SIGNATURE: u32 = u32::from_be_bytes(*b"RTAg");
/// `eventHotKeyExistsErr`: otra app ya usa ese atajo.
const ERR_EXISTS: OsStatus = -9878;

static WAKER: OnceLock<Arc<Waker>> = OnceLock::new();
/// Resultado del registro (`i32::MIN` = aún sin intentar).
static STATUS: AtomicI32 = AtomicI32::new(i32::MIN);

extern "C" fn on_hotkey(_call: Ref, _event: Ref, _user: *mut c_void) -> OsStatus {
    if let Some(waker) = WAKER.get() {
        waker.on_main_thread(sel!(globalNewAgent));
    }
    0
}

/// Registra ⌃⌥N. Hay que llamarlo una vez, desde el hilo principal.
pub fn register_new_agent(waker: Arc<Waker>) {
    if WAKER.set(waker).is_err() {
        return;
    }
    let types = EventTypeSpec {
        event_class: CLASS_KEYBOARD,
        event_kind: KIND_HOTKEY_PRESSED,
    };
    let mut handler: Ref = std::ptr::null_mut();
    let mut hotkey: Ref = std::ptr::null_mut();
    // SAFETY: llamadas a Carbon con punteros válidos durante la llamada; el
    // manejador es una función `extern "C"` estática.
    let status = unsafe {
        let status = InstallEventHandler(
            GetApplicationEventTarget(),
            on_hotkey,
            1,
            &types,
            std::ptr::null_mut(),
            &mut handler,
        );
        if status == 0 {
            RegisterEventHotKey(
                KEY_N,
                CONTROL_KEY | OPTION_KEY,
                EventHotKeyId {
                    signature: SIGNATURE,
                    id: 1,
                },
                GetApplicationEventTarget(),
                0,
                &mut hotkey,
            )
        } else {
            status
        }
    };
    STATUS.store(status, Ordering::SeqCst);
    if status != 0 {
        eprintln!("{}", describe(status));
    }
}

/// Texto para el resultado del registro (`None` si aún no se intentó).
pub fn status_text() -> Option<String> {
    match STATUS.load(Ordering::SeqCst) {
        i32::MIN => None,
        0 => Some("registrado".to_owned()),
        s => Some(describe(s)),
    }
}

fn describe(status: OsStatus) -> String {
    match status {
        ERR_EXISTS => "⌃⌥N no se pudo registrar: otra aplicación ya usa ese atajo".to_owned(),
        s => format!("⌃⌥N no se pudo registrar (error {s})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn las_constantes_de_carbon_son_las_documentadas() {
        assert_eq!(CLASS_KEYBOARD, 0x6B65_7962, "'keyb'");
        assert_eq!(CONTROL_KEY | OPTION_KEY, 0x1800);
        assert_eq!(KEY_N, 45, "kVK_ANSI_N");
    }

    #[test]
    fn el_error_de_atajo_ocupado_se_explica() {
        assert!(describe(ERR_EXISTS).contains("otra aplicación"));
        assert!(describe(-1).contains("-1"));
    }
}
