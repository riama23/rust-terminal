//! Atajo global ⌃⌥N («Nuevo agente…» desde cualquier app).
//!
//! `RegisterEventHotKey` (Carbon) es la forma de registrar un atajo global
//! que **no** pide el permiso de Accesibilidad. Los eventos llegan por el
//! run loop principal.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
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
    fn GetEventParameter(
        event: Ref,
        name: u32,
        desired_type: u32,
        actual_type: *mut u32,
        buffer_size: u32,
        actual_size: *mut u32,
        data: *mut c_void,
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
/// Código de tecla (ANSI) de la barra espaciadora.
const KEY_SPACE: u32 = 49;
const SIGNATURE: u32 = u32::from_be_bytes(*b"RTAg");
const ID_NEW_AGENT: u32 = 1;
const ID_QUICK: u32 = 2;
/// `kEventParamDirectObject` ('----') y `typeEventHotKeyID` ('hkid').
const PARAM_DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");
const TYPE_HOTKEY_ID: u32 = u32::from_be_bytes(*b"hkid");
/// `eventHotKeyExistsErr`: otra app ya usa ese atajo.
const ERR_EXISTS: OsStatus = -9878;

static WAKER: OnceLock<Arc<Waker>> = OnceLock::new();
/// Resultado del registro (`i32::MIN` = aún sin intentar).
static STATUS: AtomicI32 = AtomicI32::new(i32::MIN);
/// Lo mismo para ⌥Espacio.
static QUICK_STATUS: AtomicI32 = AtomicI32::new(i32::MIN);
static INSTALLED: AtomicBool = AtomicBool::new(false);

extern "C" fn on_hotkey(_call: Ref, event: Ref, _user: *mut c_void) -> OsStatus {
    let mut id = EventHotKeyId {
        signature: 0,
        id: 0,
    };
    // SAFETY: `event` es el evento que Carbon pasa al manejador; `id` tiene el
    // tamaño y el tipo que se piden.
    let status = unsafe {
        GetEventParameter(
            event,
            PARAM_DIRECT_OBJECT,
            TYPE_HOTKEY_ID,
            std::ptr::null_mut(),
            std::mem::size_of::<EventHotKeyId>() as u32,
            std::ptr::null_mut(),
            (&mut id as *mut EventHotKeyId).cast(),
        )
    };
    if status != 0 || id.signature != SIGNATURE {
        return 0;
    }
    if let Some(waker) = WAKER.get() {
        match id.id {
            ID_NEW_AGENT => waker.on_main_thread(sel!(globalNewAgent)),
            ID_QUICK => waker.on_main_thread(sel!(globalQuickTerminal)),
            _ => {}
        }
    }
    0
}

/// Instala el manejador de Carbon (una sola vez) y guarda el `waker`.
fn install(waker: Arc<Waker>) -> OsStatus {
    let _ = WAKER.set(waker);
    if INSTALLED.swap(true, Ordering::SeqCst) {
        return 0;
    }
    let types = EventTypeSpec {
        event_class: CLASS_KEYBOARD,
        event_kind: KIND_HOTKEY_PRESSED,
    };
    let mut handler: Ref = std::ptr::null_mut();
    // SAFETY: llamada a Carbon con punteros válidos durante la llamada; el
    // manejador es una función `extern "C"` estática.
    unsafe {
        InstallEventHandler(
            GetApplicationEventTarget(),
            on_hotkey,
            1,
            &types,
            std::ptr::null_mut(),
            &mut handler,
        )
    }
}

fn register(key_code: u32, modifiers: u32, id: u32) -> OsStatus {
    let mut hotkey: Ref = std::ptr::null_mut();
    // SAFETY: llamada a Carbon con punteros válidos durante la llamada.
    unsafe {
        RegisterEventHotKey(
            key_code,
            modifiers,
            EventHotKeyId {
                signature: SIGNATURE,
                id,
            },
            GetApplicationEventTarget(),
            0,
            &mut hotkey,
        )
    }
}

/// Registra ⌃⌥N. Hay que llamarlo una vez, desde el hilo principal.
pub fn register_new_agent(waker: Arc<Waker>) {
    let mut status = install(waker);
    if status == 0 {
        status = register(KEY_N, CONTROL_KEY | OPTION_KEY, ID_NEW_AGENT);
    }
    STATUS.store(status, Ordering::SeqCst);
    if status != 0 {
        eprintln!("{}", describe(status));
    }
}

/// Registra ⌥Espacio (terminal rápido). Una vez, desde el hilo principal.
pub fn register_quick_terminal(waker: Arc<Waker>) {
    let mut status = install(waker);
    if status == 0 {
        status = register(KEY_SPACE, OPTION_KEY, ID_QUICK);
    }
    QUICK_STATUS.store(status, Ordering::SeqCst);
    if status != 0 {
        eprintln!(
            "{}",
            if status == ERR_EXISTS {
                "⌥Espacio no se pudo registrar: otra aplicación ya usa ese atajo".to_owned()
            } else {
                format!("⌥Espacio no se pudo registrar (error {status})")
            }
        );
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
        assert_eq!(KEY_SPACE, 49, "kVK_Space");
        assert_eq!(OPTION_KEY, 0x0800, "optionKey");
        assert_eq!(PARAM_DIRECT_OBJECT, 0x2D2D_2D2D);
        assert_eq!(TYPE_HOTKEY_ID, 0x686B_6964, "'hkid'");
    }

    #[test]
    fn el_error_de_atajo_ocupado_se_explica() {
        assert!(describe(ERR_EXISTS).contains("otra aplicación"));
        assert!(describe(-1).contains("-1"));
    }
}
