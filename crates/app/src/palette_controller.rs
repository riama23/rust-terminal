//! La paleta de comandos (⌘⇧P): abrirla, reunir lo que se busca (acciones del
//! menú e historial), ejecutar o insertar, y la frecuencia de uso.
//!
//! **Lo que se escribe en la PTY pasa siempre por `command_palette::insertable`**
//! y solo si el panel es una shell libre (ni un agente ni un programa en marcha).

use std::cell::RefCell;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::atomic::Ordering::SeqCst;
use std::time::{SystemTime, UNIX_EPOCH};

use objc2::rc::{Retained, Weak};
use objc2::runtime::Sel;
use objc2::{MainThreadOnly, sel};
use objc2_app_kit::{NSApplication, NSAutoresizingMaskOptions};

use crate::command_palette::{self as cp, Frecency, Item, Kind, Scope};
use crate::palette_ui::{MAX_ROWS, PaletteRow, PaletteView};
use crate::view::TerminalView;

/// Historial de zsh: se lee solo el final del archivo.
const HISTORY_TAIL_BYTES: u64 = 256 * 1024;
/// Comandos de historial que se ofrecen como máximo.
const MAX_HISTORY: usize = 3000;

#[derive(Default)]
pub struct PaletteController {
    inner: RefCell<Inner>,
}

#[derive(Default)]
struct Inner {
    view: Option<Retained<PaletteView>>,
    items: Vec<Item>,
    actions: Vec<(String, Sel)>,
    /// Lo que se está enseñando, fila a fila.
    shown: Vec<Item>,
    /// Comando de cada flujo, por su clave.
    flows: Vec<(String, String)>,
    frecency: Frecency,
}

/// Las acciones del menú: título, atajo y selector.
fn action_table() -> Vec<(&'static str, &'static str, Sel)> {
    vec![
        ("Nueva pestaña", "⌘T", sel!(newTab:)),
        ("Nuevo agente…", "⌘N", sel!(newAgent:)),
        ("Revisar cambios del agente", "⌘⇧R", sel!(reviewChanges:)),
        ("Dividir en vertical", "⌘D", sel!(splitVertical:)),
        ("Dividir en horizontal", "⌘⇧D", sel!(splitHorizontal:)),
        ("Cerrar el panel", "⌘W", sel!(closePane:)),
        ("Cerrar la pestaña", "⌘⇧W", sel!(closeTab:)),
        ("Pestaña anterior", "⌘⇧[", sel!(previousTab:)),
        ("Pestaña siguiente", "⌘⇧]", sel!(nextTab:)),
        ("Ampliar o restaurar el panel", "⌘⇧↵", sel!(toggleZoom:)),
        ("Panel de la izquierda", "⌘⌥←", sel!(focusLeft:)),
        ("Panel de la derecha", "⌘⌥→", sel!(focusRight:)),
        ("Panel de arriba", "⌘⌥↑", sel!(focusUp:)),
        ("Panel de abajo", "⌘⌥↓", sel!(focusDown:)),
        ("Mover el divisor a la izquierda", "⌘⌃←", sel!(resizeLeft:)),
        ("Mover el divisor a la derecha", "⌘⌃→", sel!(resizeRight:)),
        ("Mover el divisor hacia arriba", "⌘⌃↑", sel!(resizeUp:)),
        ("Mover el divisor hacia abajo", "⌘⌃↓", sel!(resizeDown:)),
        ("Bloque anterior", "⌘↑", sel!(previousBlock:)),
        ("Bloque siguiente", "⌘↓", sel!(nextBlock:)),
        ("Relanzar el bloque seleccionado", "", sel!(blockRerun:)),
        (
            "Relanzar solo los tests fallidos del bloque",
            "",
            sel!(blockRerunFailed:),
        ),
        (
            "Copiar la salida del bloque seleccionado",
            "",
            sel!(blockCopy:),
        ),
        ("Detener el bloque en ejecución", "", sel!(blockStop:)),
        (
            "Colapsar o expandir la salida del bloque",
            "",
            sel!(blockCollapse:),
        ),
        (
            "Abrir el error del bloque en el editor",
            "⌘O",
            sel!(blockOpen:),
        ),
        (
            "Copiar la salida del último comando",
            "⌘⇧C",
            sel!(copyLastOutput:),
        ),
        ("Informar de un fallo…", "⌘⇧I", sel!(reportIssue:)),
    ]
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn frecency_path() -> PathBuf {
    let config = crate::config::Config::path();
    config
        .parent()
        .map(|p| p.join("palette-frecency.tsv"))
        .unwrap_or_else(|| PathBuf::from("palette-frecency.tsv"))
}

fn workflows_path() -> PathBuf {
    crate::config::Config::path()
        .parent()
        .map(|p| p.join("workflows.toml"))
        .unwrap_or_else(|| PathBuf::from("workflows.toml"))
}

fn load_frecency() -> Frecency {
    std::fs::read_to_string(frecency_path())
        .map(|t| Frecency::parse(&t))
        .unwrap_or_default()
}

/// Guarda la frecuencia (0600, de forma atómica). Los fallos se ignoran: la
/// paleta funciona igual sin recordar.
fn save_frecency(f: &Frecency) {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let path = frecency_path();
    let Some(dir) = path.parent() else { return };
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let tmp = path.with_extension("tmp");
    let _ = std::fs::remove_file(&tmp);
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)
        .and_then(|mut f2| f2.write_all(f.serialize().as_bytes()));
    if written.is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    } else {
        let _ = std::fs::remove_file(&tmp);
    }
}

/// El final del historial de zsh (`$HISTFILE` o `~/.zsh_history`).
fn read_zsh_history() -> Vec<(Option<u64>, String)> {
    let path = std::env::var_os("HISTFILE")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".zsh_history")));
    let Some(path) = path else { return Vec::new() };
    let Ok(mut file) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let len = file.metadata().map_or(0, |m| m.len());
    let start = len.saturating_sub(HISTORY_TAIL_BYTES);
    if file.seek(SeekFrom::Start(start)).is_err() {
        return Vec::new();
    }
    let mut bytes = Vec::new();
    if file.read_to_end(&mut bytes).is_err() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&bytes);
    // Si se empezó a mitad de archivo, la primera línea está cortada.
    let text = if start > 0 {
        text.split_once('\n').map_or("", |(_, rest)| rest)
    } else {
        &text
    };
    cp::parse_zsh_history(text)
}

fn tilde(path: &str) -> String {
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && path.starts_with(&home) => {
            format!("~{}", &path[home.len()..])
        }
        _ => path.to_owned(),
    }
}

impl TerminalView {
    pub(crate) fn palette_ctl(&self) -> &PaletteController {
        self.ivars_palette()
    }

    /// ⌘⇧P: abre la paleta (o la cierra si ya estaba).
    pub fn toggle_palette(&self) {
        if self.palette_ctl().inner.borrow().view.is_some() {
            self.close_palette();
        } else {
            self.open_palette("");
        }
    }

    pub fn open_palette(&self, query: &str) {
        let Some(container) = self.container() else {
            return;
        };
        let (items, actions) = self.palette_items();
        {
            let mut inner = self.palette_ctl().inner.borrow_mut();
            inner.items = items;
            inner.actions = actions;
            inner.frecency = load_frecency();
        }
        let view = PaletteView::new(self.mtm(), container.bounds());
        view.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        let me: Retained<TerminalView> = objc2::rc::Retained::from(self);
        let weak: Weak<TerminalView> = Weak::from_retained(&me);
        let (w1, w2, w3) = (weak.clone(), weak.clone(), weak);
        view.set_handlers(
            move |i, insert| {
                if let Some(v) = w1.load() {
                    v.palette_activate(i, insert);
                }
            },
            move |q, scope| {
                if let Some(v) = w2.load() {
                    v.palette_refresh(q, scope);
                }
            },
            move || {
                if let Some(v) = w3.load() {
                    v.close_palette();
                }
            },
        );
        view.set_query(query);
        container.addSubview(&view);
        if let Some(window) = self.window() {
            window.makeFirstResponder(Some(&view));
        }
        self.palette_ctl().inner.borrow_mut().view = Some(view);
        self.palette_refresh(query, Scope::All);
    }

    pub fn close_palette(&self) {
        let view = self.palette_ctl().inner.borrow_mut().view.take();
        if let Some(view) = view {
            view.removeFromSuperview();
        }
        // El teclado vuelve a la terminal.
        if let Some(window) = self.window() {
            window.makeFirstResponder(Some(self));
        }
    }

    /// Acciones del menú e historial (comandos de los bloques y de zsh).
    fn palette_items(&self) -> (Vec<Item>, Vec<(String, Sel)>) {
        let mut items = Vec::new();
        let mut actions = Vec::new();
        let (flows, problems) = std::fs::read_to_string(workflows_path())
            .map(|t| cp::parse_workflows(&t))
            .unwrap_or_default();
        for p in &problems {
            eprintln!("workflows.toml: {p}");
        }
        {
            let mut inner = self.palette_ctl().inner.borrow_mut();
            inner.flows.clear();
            for f in flows {
                let key = format!("flow:{}", f.name);
                inner.flows.push((key.clone(), f.command.clone()));
                items.push(Item {
                    kind: Kind::Flow,
                    key,
                    label: f.name,
                    detail: if f.description.is_empty() {
                        f.command
                    } else {
                        format!("{} · {}", f.description, f.command)
                    },
                });
            }
        }
        for (label, shortcut, selector) in action_table() {
            let key = format!("act:{}", selector.name().to_string_lossy());
            items.push(Item {
                kind: Kind::Action,
                key: key.clone(),
                label: label.to_owned(),
                detail: if shortcut.is_empty() {
                    "Sin atajo".to_owned()
                } else {
                    shortcut.to_owned()
                },
            });
            actions.push((key, selector));
        }

        // Comandos de esta sesión (bloques de todos los paneles), recientes primero.
        let mut recent: Vec<(u64, String, String)> = Vec::new();
        for session in self.all_sessions() {
            let term = session.term.lock().unwrap();
            for b in term.blocks() {
                let (Some(cmd), Some(_), Some(finished)) =
                    (b.command.as_deref(), b.output, b.finished_at)
                else {
                    continue;
                };
                if !cp::insertable(cmd) {
                    continue;
                }
                let age = finished.elapsed().as_secs();
                let status = match b.exit_code {
                    Some(0) => "✓".to_owned(),
                    Some(c) => format!("✕ {c}"),
                    None => String::new(),
                };
                let cwd = b.cwd.as_deref().map(tilde).unwrap_or_default();
                let detail = [cwd, cp::age_label(age), status]
                    .into_iter()
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join(" · ");
                recent.push((age, cmd.trim().to_owned(), detail));
            }
        }
        recent.sort_by_key(|(age, _, _)| *age);
        let mut seen = std::collections::HashSet::new();
        for (_, cmd, detail) in recent {
            if seen.insert(cmd.clone()) {
                items.push(Item {
                    kind: Kind::History,
                    key: format!("cmd:{cmd}"),
                    label: cmd,
                    detail,
                });
            }
        }
        let now = now_secs();
        for (ts, cmd) in read_zsh_history().into_iter().take(MAX_HISTORY) {
            if seen.insert(cmd.clone()) {
                items.push(Item {
                    kind: Kind::History,
                    key: format!("cmd:{cmd}"),
                    label: cmd,
                    detail: ts.map_or_else(
                        || "Historial de zsh".to_owned(),
                        |t| {
                            format!(
                                "Historial de zsh · {}",
                                cp::age_label(now.saturating_sub(t))
                            )
                        },
                    ),
                });
            }
        }
        (items, actions)
    }

    /// Recalcula las filas para la consulta y el ámbito actuales.
    fn palette_refresh(&self, query: &str, scope: Scope) {
        let (rows, view) = {
            let mut inner = self.palette_ctl().inner.borrow_mut();
            let ranked = cp::rank(
                &inner.items,
                query,
                scope,
                &inner.frecency,
                now_secs(),
                MAX_ROWS,
            );
            let mut shown: Vec<Item> = ranked.iter().map(|&i| inner.items[i].clone()).collect();
            // Vacío: se ofrece ejecutar lo escrito como comando.
            let q = query.trim();
            if shown.is_empty() && scope != Scope::Actions && cp::insertable(q) {
                shown.push(Item {
                    kind: Kind::History,
                    key: format!("cmd:{q}"),
                    label: format!("Ejecutar «{q}» como comando"),
                    detail: "Sin coincidencias · ↵ ejecuta, ⌘↵ inserta".to_owned(),
                });
            }
            let rows: Vec<PaletteRow> = shown
                .iter()
                .map(|it| PaletteRow {
                    tag: it.kind.tag(),
                    label: it.label.clone(),
                    detail: it.detail.clone(),
                })
                .collect();
            inner.shown = shown;
            (rows, inner.view.clone())
        };
        if let Some(view) = view {
            view.set_rows(rows);
            if let Some(dir) = std::env::var_os("RT_SNAPSHOT_DIR") {
                let dir = PathBuf::from(dir);
                let _ = std::fs::create_dir_all(&dir);
                crate::agent_ui::snapshot_png(&view, &dir.join("palette.png").to_string_lossy());
            }
        }
    }

    /// ↵ ejecuta y ⌘↵ inserta sin ejecutar.
    fn palette_activate(&self, index: usize, insert: bool) {
        let (item, action) = {
            let inner = self.palette_ctl().inner.borrow();
            let Some(item) = inner.shown.get(index).cloned() else {
                return;
            };
            let action = inner
                .actions
                .iter()
                .find(|(k, _)| *k == item.key)
                .map(|(_, s)| *s);
            (item, action)
        };
        self.close_palette();
        match (item.kind, action) {
            (Kind::Action, Some(selector)) => {
                self.palette_record(&item.key);
                // SAFETY: `selector` es una acción del menú; con destino `None`
                // la recibe el primero de la cadena de respuesta que la entienda.
                unsafe {
                    NSApplication::sharedApplication(self.mtm())
                        .sendAction_to_from(selector, None, None);
                }
            }
            (Kind::History | Kind::Flow, _) => {
                let command = if item.kind == Kind::Flow {
                    self.palette_ctl()
                        .inner
                        .borrow()
                        .flows
                        .iter()
                        .find(|(k, _)| *k == item.key)
                        .map(|(_, c)| c.clone())
                        .unwrap_or_default()
                } else {
                    item.key.strip_prefix("cmd:").unwrap_or("").to_owned()
                };
                if self.palette_send(&command, insert) {
                    self.palette_record(&item.key);
                }
            }
            _ => {}
        }
    }

    fn palette_record(&self, key: &str) {
        let mut inner = self.palette_ctl().inner.borrow_mut();
        inner.frecency.record(key, now_secs());
        save_frecency(&inner.frecency);
    }

    /// Escribe `command` en la shell del panel con el foco. `false` si no es
    /// seguro (agente, programa en marcha o caracteres de control).
    fn palette_send(&self, command: &str, insert: bool) -> bool {
        let Some(session) = self.focused_session() else {
            return false;
        };
        let focused = self.focused_pane();
        let is_agent = self.agents().hub_ref().session_of_pane(focused).is_some();
        if is_agent || session.busy.load(SeqCst) || session.is_busy() {
            self.info(
                "No se puede escribir aquí",
                "El panel con el foco es un agente o tiene un programa en marcha: el comando iría a ese programa, no a la shell.",
            );
            return false;
        }
        if !cp::insertable(command) {
            self.info(
                "Comando no válido",
                "Tiene caracteres de control (por ejemplo un salto de línea): no se puede escribir con seguridad.",
            );
            return false;
        }
        let mut bytes = command.trim().as_bytes().to_vec();
        if !insert {
            bytes.push(b'\r');
        }
        session.write(&bytes);
        true
    }

    /// Solo en depuración (`RT_PALETTE_KEYS`): entrega teclas sintéticas a la
    /// vista de la paleta por su `keyDown:` real, sin necesitar el foco de la
    /// pantalla. `\n` es Intro, `\t` es Tab, `\u{1b}` es Esc, `↑` y `↓` las flechas
    /// y `⌘\n` (el carácter `\u{2318}` antes de `\n`) inserta sin ejecutar.
    #[cfg(debug_assertions)]
    pub fn debug_palette_keys(&self, keys: &str) {
        use objc2_app_kit::{NSEvent, NSEventModifierFlags, NSEventType};
        use objc2_foundation::{NSPoint, NSString};
        let Some(view) = self.palette_ctl().inner.borrow().view.clone() else {
            return;
        };
        let mut chars = keys.chars().peekable();
        while let Some(c) = chars.next() {
            let (text, code, flags) = match c {
                '\u{2318}' => {
                    chars.next();
                    ("\r", 36, NSEventModifierFlags::Command)
                }
                '\n' => ("\r", 36, NSEventModifierFlags::empty()),
                '\t' => ("\t", 48, NSEventModifierFlags::empty()),
                '\u{1b}' => ("\u{1b}", 53, NSEventModifierFlags::empty()),
                '↑' => ("", 126, NSEventModifierFlags::empty()),
                '↓' => ("", 125, NSEventModifierFlags::empty()),
                other => {
                    let s = other.to_string();
                    let ns = NSString::from_str(&s);
                    if let Some(ev) = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
                        NSEventType::KeyDown, NSPoint::ZERO, NSEventModifierFlags::empty(), 0.0, 0, None, &ns, &ns, false, 0,
                    ) {
                        view.keyDown(&ev);
                    }
                    continue;
                }
            };
            let ns = NSString::from_str(text);
            if let Some(ev) = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
                NSEventType::KeyDown, NSPoint::ZERO, flags, 0.0, 0, None, &ns, &ns, false, code,
            ) {
                view.keyDown(&ev);
            }
            // Si la paleta se cerró (↵ o Esc), no hay más teclas para ella.
            if self.palette_ctl().inner.borrow().view.is_none() {
                break;
            }
        }
    }
}
