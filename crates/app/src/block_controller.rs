//! Bloques visuales (v1.5): selección con ⌘↑/⌘↓ y las acciones de la barra
//! del bloque (detener, relanzar, copiar la salida y abrir el error en el
//! editor). Las órdenes que se escriben en la PTY pasan por `blocks::rerun_bytes`.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering::SeqCst;

use rust_terminal_core::Block;

use crate::block_ui::BlockBarInfo;
use crate::blocks::{self, Step};
use crate::session::Session;
use crate::view::TerminalView;

/// Tope de salida que se escanea buscando `archivo:línea` (1 MiB).
const MAX_SCAN: usize = 1024 * 1024;

impl TerminalView {
    /// ⌘↑ / ⌘↓: salta al bloque anterior o siguiente, lo selecciona y lo
    /// enseña. Sin bloque siguiente, vuelve al final y quita la selección.
    pub fn step_block(&self, step: Step) {
        let Some(session) = self.focused_session() else {
            return;
        };
        let pane = self.focused_pane();
        let (visual, _) = self.block_config();
        let current = self
            .block_selected()
            .filter(|(p, _)| *p == pane)
            .map(|(_, id)| id);
        let picked = {
            let mut term = session.term.lock().unwrap();
            // Solo comandos ejecutados: el prompt actual no es un bloque.
            let list: Vec<(u64, u64)> = term
                .blocks()
                .iter()
                .filter(|b| b.output.is_some())
                .map(|b| (b.id, b.prompt.line))
                .collect();
            let ids: Vec<u64> = list.iter().map(|(id, _)| *id).collect();
            let target = blocks::step_block(&ids, current, step);
            match target {
                Some(id) => {
                    if let Some(line) = list.iter().find(|(b, _)| *b == id).map(|(_, l)| *l) {
                        term.scroll_to_line(line);
                    }
                }
                None if step == Step::Next => term.scroll_to_bottom(),
                None => {}
            }
            target
        };
        if visual {
            let new = match (picked, step) {
                (Some(id), _) => Some((pane, id)),
                // Al pasar del último, se quita la selección.
                (None, Step::Next) => None,
                (None, Step::Previous) => current.map(|id| (pane, id)),
            };
            self.set_block_selected(new);
        }
        self.update_layout();
    }

    /// Quita la selección de bloque (al teclear, por ejemplo). `true` si había.
    pub fn clear_block_selection(&self) -> bool {
        let had = self.block_selected().is_some();
        self.set_block_selected(None);
        if had {
            self.update_layout();
        }
        had
    }

    /// El bloque seleccionado del panel con el foco, con su sesión.
    fn selected_block_of_focus(&self) -> Option<(Arc<Session>, Block)> {
        let (pane, id) = self.block_selected()?;
        if pane != self.focused_pane() {
            return None;
        }
        let session = self.session(pane)?;
        let block = {
            let term = session.term.lock().unwrap();
            if term.modes().alt_screen {
                return None;
            }
            term.blocks().iter().find(|b| b.id == id)?.clone()
        };
        Some((session, block))
    }

    fn block_output_text(session: &Session, block: &Block) -> Option<String> {
        session.term.lock().unwrap().block_output(block)
    }

    fn location_of(&self, session: &Session, block: &Block) -> Option<blocks::Location> {
        let mut output = Self::block_output_text(session, block)?;
        if output.len() > MAX_SCAN {
            let mut cut = MAX_SCAN;
            while !output.is_char_boundary(cut) {
                cut -= 1;
            }
            output.truncate(cut);
        }
        let cwd = block.cwd.as_deref().map(PathBuf::from);
        blocks::first_location(&output, cwd.as_deref(), |p| p.is_file())
    }

    /// Lo que enseña la barra del bloque seleccionado (`None`: sin barra).
    pub fn block_bar_info(&self) -> Option<BlockBarInfo> {
        if !self.block_config().0 {
            return None;
        }
        let (session, block) = self.selected_block_of_focus()?;
        let running = block.output.is_some() && !block.is_finished();
        let failed = block.exit_code.is_some_and(|c| c != 0);
        let command = block.command.clone().unwrap_or_default();
        let status = if running {
            "◐ en ejecución".to_owned()
        } else {
            let time = block
                .duration()
                .map(|d| format!(" · {}", blocks::format_duration(d)))
                .unwrap_or_default();
            match block.exit_code {
                Some(0) => format!("✓ correcto{time}"),
                Some(code) => format!("✕ código {code}{time}"),
                None => format!("· sin código de salida{time}"),
            }
        };
        let label = if command.is_empty() {
            status
        } else {
            format!("{status}  ·  {command}")
        };
        Some(BlockBarInfo {
            label,
            failed,
            running,
            can_rerun: !running
                && !session.busy.load(SeqCst)
                && blocks::rerun_bytes(&command).is_some(),
            can_open: failed && !running && self.location_of(&session, &block).is_some(),
        })
    }

    /// «Copiar salida» del bloque seleccionado.
    pub fn block_copy(&self) {
        let Some((session, block)) = self.selected_block_of_focus() else {
            return;
        };
        if let Some(text) = Self::block_output_text(&session, &block) {
            crate::view::copy_to_pasteboard(&text);
        }
    }

    /// «Relanzar»: escribe el comando del bloque en la PTY, solo con la shell libre.
    pub fn block_rerun(&self) {
        let Some((session, block)) = self.selected_block_of_focus() else {
            return;
        };
        let running = block.output.is_some() && !block.is_finished();
        if running || session.busy.load(SeqCst) {
            return;
        }
        if let Some(bytes) = block.command.as_deref().and_then(blocks::rerun_bytes) {
            session.write(&bytes);
            // La salida nueva es otro bloque: se vuelve al final.
            session.term.lock().unwrap().scroll_to_bottom();
            self.set_block_selected(None);
            self.update_layout();
        }
    }

    /// «Detener»: Ctrl-C a la PTY, solo con el bloque en ejecución.
    pub fn block_stop(&self) {
        let Some((session, block)) = self.selected_block_of_focus() else {
            return;
        };
        if block.output.is_some() && !block.is_finished() {
            session.write(&[0x03]);
        }
    }

    /// ⌘O: abre el primer `archivo:línea:columna` de la salida del bloque
    /// con el editor de `blocks.editor`.
    pub fn block_open_in_editor(&self) {
        let Some((session, block)) = self.selected_block_of_focus() else {
            return;
        };
        let Some(loc) = self.location_of(&session, &block) else {
            return;
        };
        let template = self.block_config().1;
        let args = blocks::editor_args(&template, &loc);
        // Por la shell de login para tener el PATH del usuario (`code`, `zed`…).
        // Las palabras van como argumentos, nunca interpretadas por la shell.
        let mut command =
            std::process::Command::new(crate::agent::launch::agent_shell(&self.user_shell()));
        command
            .args(["-l", "-c", "exec \"$@\"", "editor"])
            .args(&args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        if let Some(dir) = loc.path.parent() {
            command.current_dir(dir);
        }
        match command.spawn() {
            Ok(mut child) => {
                // Sin zombis: alguien tiene que esperar al proceso.
                let _ = std::thread::Builder::new()
                    .name("editor-wait".into())
                    .spawn(move || {
                        let _ = child.wait();
                    });
            }
            Err(e) => self.info("No se pudo abrir el editor", &e.to_string()),
        }
    }
}
