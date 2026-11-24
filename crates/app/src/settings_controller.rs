//! La ventana de ajustes (⌘,): cada fila es una opción de `config::OPTIONS` y
//! escribir en ella cambia la línea de `config.toml` (el vigilante recarga y
//! aplica). Se conservan los comentarios del archivo (`config_edit`).

use std::cell::RefCell;
use std::path::Path;

use objc2::rc::{Retained, Weak};
use objc2::runtime::AnyObject;
use objc2::{ClassType, MainThreadOnly, msg_send, sel};
use objc2_app_kit::{
    NSBackingStoreType, NSButton, NSControlStateValueOn, NSPopUpButton, NSTextField, NSWindow,
    NSWindowStyleMask,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use crate::config::{self, Config, Kind, OPTIONS};
use crate::config_edit;
use crate::settings_ui::{
    self, Control, PAGES, RowSpec, SettingsActions, SettingsRoot, WINDOW_H, WINDOW_W,
};
use crate::view::TerminalView;

#[derive(Default)]
pub struct SettingsController {
    inner: RefCell<Option<State>>,
}

struct State {
    window: Retained<NSWindow>,
    root: Retained<SettingsRoot>,
    page: usize,
}

fn actions() -> SettingsActions {
    SettingsActions {
        toggle: sel!(settingsToggle:),
        commit: sel!(settingsCommit:),
        reset: sel!(settingsReset:),
        open_file: sel!(settingsOpenFile:),
    }
}

/// El archivo de configuración como tabla TOML (vacía si no existe o está roto).
fn read_table(path: &Path) -> toml::Table {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| t.parse().ok())
        .unwrap_or_default()
}

/// Escribe `texto` en `path` de forma atómica (el vigilante entiende el reemplazo).
fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

/// Las filas de una página, con los valores actuales.
fn rows_for(page: usize, table: &toml::Table) -> Vec<RowSpec> {
    let sections = PAGES[page].1;
    OPTIONS
        .iter()
        .enumerate()
        .filter(|(_, o)| sections.contains(&o.section))
        .map(|(index, o)| {
            let value = config::display_value(o, table);
            let control = match o.kind {
                Kind::Bool => Control::Switch(value == "true"),
                Kind::Choice(list) => Control::Choice(
                    list.iter().map(|s| (*s).to_owned()).collect(),
                    list.iter().position(|s| *s == value).unwrap_or(0),
                ),
                Kind::Palette => {
                    Control::ReadOnly("16 colores: se editan en config.toml".to_owned())
                }
                _ => Control::Text(value),
            };
            RowSpec {
                index,
                label: o.description.to_owned(),
                path: format!("{}.{}", o.section, o.key),
                control,
                overridden: config::is_set(o, table),
            }
        })
        .collect()
}

impl TerminalView {
    fn settings_state(&self) -> &RefCell<Option<State>> {
        &self.ivars_settings().inner
    }

    /// ⌘,: abre la ventana de ajustes (o la trae al frente).
    pub fn open_settings(&self) {
        if let Some(state) = self.settings_state().borrow().as_ref() {
            #[allow(deprecated)]
            objc2_app_kit::NSApplication::sharedApplication(self.mtm())
                .activateIgnoringOtherApps(true);
            state.window.makeKeyAndOrderFront(None);
            return;
        }
        let mtm = self.mtm();
        let me: Retained<TerminalView> = Retained::from(self);
        let root = SettingsRoot::new(mtm, me.as_ref(), actions());
        let frame = NSRect::new(NSPoint::ZERO, NSSize::new(WINDOW_W, WINDOW_H));
        // SAFETY: ventana propia, que no se libera al cerrarse.
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                frame,
                // Tamaño fijo (960 × 680 según el spec): sin `Resizable`.
                NSWindowStyleMask::Titled
                    | NSWindowStyleMask::Closable
                    | NSWindowStyleMask::Miniaturizable,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        // SAFETY: la ventana no es de un controlador de ventana.
        unsafe { window.setReleasedWhenClosed(false) };
        window.setTitle(&NSString::from_str("Ajustes"));
        window.setContentView(Some(&root));
        window.center();
        let weak: Weak<TerminalView> = Weak::from_retained(&me);
        root.set_on_page(move |i| {
            if let Some(v) = weak.load() {
                v.settings_show_page(i);
            }
        });
        *self.settings_state().borrow_mut() = Some(State {
            window: window.clone(),
            root,
            page: 0,
        });
        self.settings_show_page(0);
        #[allow(deprecated)]
        objc2_app_kit::NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
        window.makeKeyAndOrderFront(None);
    }

    /// Solo en depuración (`RT_SETTINGS_CLICK=sección.clave`): pulsa el control
    /// de esa opción de la página abierta.
    #[cfg(debug_assertions)]
    pub fn debug_settings_click(&self, path: &str) {
        let Some(index) = OPTIONS
            .iter()
            .position(|o| format!("{}.{}", o.section, o.key) == path)
        else {
            return;
        };
        let page = PAGES
            .iter()
            .position(|(_, secs)| secs.contains(&OPTIONS[index].section))
            .unwrap_or(0);
        self.settings_show_page(page);
        let root = self
            .settings_state()
            .borrow()
            .as_ref()
            .map(|s| s.root.clone());
        if let Some(root) = root {
            root.debug_click(index as isize);
        }
    }

    pub(crate) fn settings_show_page(&self, page: usize) {
        let Some((root, mtm)) = ({
            let mut slot = self.settings_state().borrow_mut();
            slot.as_mut().map(|s| {
                s.page = page.min(PAGES.len() - 1);
                (s.root.clone(), self.mtm())
            })
        }) else {
            return;
        };
        let page = page.min(PAGES.len() - 1);
        let table = read_table(&Config::path());
        root.select_page(page);
        root.set_header(
            PAGES[page].0,
            "Cada ajuste es una línea de config.toml: lo que cambies aquí se escribe en el archivo.",
        );
        let me: Retained<TerminalView> = Retained::from(self);
        root.show_rows(mtm, &rows_for(page, &table), me.as_ref(), actions());
        if let Some(dir) = std::env::var_os("RT_SNAPSHOT_DIR") {
            let dir = std::path::PathBuf::from(dir);
            let _ = std::fs::create_dir_all(&dir);
            crate::agent_ui::snapshot_png(&root, &dir.join("settings.png").to_string_lossy());
        }
    }

    /// Escribe (o quita, con `None`) la opción `index` en `config.toml`.
    fn settings_write(&self, index: usize, literal: Option<&str>) {
        let Some(spec) = OPTIONS.get(index) else {
            return;
        };
        let path = Config::path();
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let new = match literal {
            Some(l) => config_edit::set_value(&text, spec.section, spec.key, l),
            None => config_edit::remove_value(&text, spec.section, spec.key),
        };
        // No se escribe nada que deje el archivo ilegible.
        if let Err(e) = Config::parse(&new) {
            self.settings_status(&format!("No se guardó: {e}"), true);
            return;
        }
        match write_atomic(&path, &new) {
            Ok(()) => self.settings_status(
                &format!(
                    "Guardado en {} · {}.{}",
                    path.display(),
                    spec.section,
                    spec.key
                ),
                false,
            ),
            Err(e) => self.settings_status(&format!("No se pudo escribir: {e}"), true),
        }
    }

    fn settings_status(&self, text: &str, error: bool) {
        if let Some(s) = self.settings_state().borrow().as_ref() {
            s.root.set_status(text, error);
        }
    }

    pub fn settings_toggle(&self, sender: &AnyObject) {
        let index = settings_ui::tag_of(sender);
        // SAFETY: el remitente es un `NSButton` (casilla).
        let state: isize = unsafe { msg_send![sender, state] };
        if index >= 0 {
            self.settings_write(
                index as usize,
                Some(if state == NSControlStateValueOn {
                    "true"
                } else {
                    "false"
                }),
            );
            // El título de la casilla cambia con su estado.
            let b: &NSButton = unsafe { &*(sender as *const AnyObject as *const NSButton) };
            b.setTitle(&NSString::from_str(if state == NSControlStateValueOn {
                "Activado"
            } else {
                "Desactivado"
            }));
        }
    }

    /// Un campo de texto o un menú desplegable terminó de editarse.
    pub fn settings_commit(&self, sender: &AnyObject) {
        let index = settings_ui::tag_of(sender);
        let Some(spec) = usize::try_from(index).ok().and_then(|i| OPTIONS.get(i)) else {
            return;
        };
        // SAFETY: `isKindOfClass:` existe en todo NSObject.
        let is_popup: bool = unsafe { msg_send![sender, isKindOfClass: NSPopUpButton::class()] };
        let input = if is_popup {
            let p: &NSPopUpButton =
                unsafe { &*(sender as *const AnyObject as *const NSPopUpButton) };
            p.titleOfSelectedItem()
                .map(|s| s.to_string())
                .unwrap_or_default()
        } else {
            let f: &NSTextField = unsafe { &*(sender as *const AnyObject as *const NSTextField) };
            f.stringValue().to_string()
        };
        // Un valor que no cambia no se escribe: al cambiar de página, el campo
        // que tenía el foco avisa al desaparecer y no debe tocar el archivo.
        if input.trim() == config::display_value(spec, &read_table(&Config::path())) {
            return;
        }
        // Un campo vacío de una opción de texto vale «vacío»; de una de otro tipo, es un error.
        match config::literal_for(spec, &input) {
            Ok(literal) => self.settings_write(index as usize, Some(&literal)),
            Err(e) => self.settings_status(&format!("{}.{}: {e}", spec.section, spec.key), true),
        }
    }

    pub fn settings_reset(&self, sender: &AnyObject) {
        let index = settings_ui::tag_of(sender);
        if index >= 0 {
            self.settings_write(index as usize, None);
            let page = self
                .settings_state()
                .borrow()
                .as_ref()
                .map_or(0, |s| s.page);
            self.settings_show_page(page);
        }
    }

    /// «Abrir config.toml»: lo crea vacío si no existe y lo abre en el editor de texto.
    pub fn settings_open_file(&self) {
        let path = Config::path();
        if !path.exists() {
            let _ = write_atomic(&path, "");
        }
        let _ = std::process::Command::new("/usr/bin/open")
            .arg("-t")
            .arg(&path)
            .status();
    }
}
