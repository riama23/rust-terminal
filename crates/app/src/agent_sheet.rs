//! Formulario de nuevo agente (⌘N): un `NSAlert` con una vista accesoria.
//!
//! Principio "no magic": el recuadro *Ejecuta exactamente* se actualiza al
//! teclear y muestra la línea de `claude` que se va a lanzar. La lógica (el
//! slug, el spec, la validación) está en `agent::launch` y se prueba allí.

use std::cell::RefCell;
use std::path::Path;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol};
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSButton, NSControlTextDidChangeNotification, NSFont,
    NSPopUpButton, NSTextField, NSView,
};
use objc2_foundation::{
    MainThreadMarker, NSNotificationCenter, NSObject, NSPoint, NSRect, NSSize, NSString,
};

use crate::agent::launch::{self, Form, MODELS};

const WIDTH: f64 = 460.0;
const LABEL_W: f64 = 148.0;

pub struct HelperIvars {
    task: Retained<NSTextField>,
    name: Retained<NSTextField>,
    isolation: Retained<NSPopUpButton>,
    mode: Retained<NSPopUpButton>,
    model: Retained<NSPopUpButton>,
    preview: Retained<NSTextField>,
    /// El botón «Lanzar» del alert: solo está activo si el formulario es válido.
    launch: RefCell<Option<Retained<NSButton>>>,
}

define_class!(
    // SAFETY: NSObject no impone requisitos a sus subclases y no tiene `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "RTAgentSheetHelper"]
    #[ivars = HelperIvars]
    pub struct Helper;

    unsafe impl NSObjectProtocol for Helper {}

    impl Helper {
        /// Cambió un campo (texto o desplegable): rehace el recuadro de la orden.
        #[unsafe(method(refresh:))]
        fn refresh(&self, _sender: Option<&AnyObject>) {
            self.update();
        }
    }
);

impl Helper {
    fn form(&self) -> Form {
        let iv = self.ivars();
        Form {
            task: iv.task.stringValue().to_string(),
            new_worktree: iv.isolation.indexOfSelectedItem() == 0,
            worktree_name: iv.name.stringValue().to_string(),
            mode_index: iv.mode.indexOfSelectedItem().max(0) as usize,
            model_index: iv.model.indexOfSelectedItem().max(0) as usize,
        }
    }

    /// `true` si el formulario se puede lanzar tal como está.
    fn update(&self) -> bool {
        let iv = self.ivars();
        let form = self.form();
        let spec = form.to_spec();
        iv.name.setEnabled(form.new_worktree);
        iv.name
            .setPlaceholderString(Some(&NSString::from_str(&launch::slug(&form.task))));
        let (text, ok) = match launch::command_args(&spec, Path::new("…/settings-<agente>.json"))
        {
            Ok(args) => (
                format!("Ejecuta exactamente:\n{}", launch::display_command(&args)),
                true,
            ),
            Err(e) => (format!("⚠ {e}"), false),
        };
        iv.preview.setStringValue(&NSString::from_str(&text));
        if let Some(button) = iv.launch.borrow().as_ref() {
            button.setEnabled(ok);
        }
        ok
    }
}

fn label(mtm: MainThreadMarker, text: &str, frame: NSRect) -> Retained<NSTextField> {
    let l = NSTextField::labelWithString(&NSString::from_str(text), mtm);
    l.setFont(Some(&NSFont::systemFontOfSize(12.0)));
    l.setFrame(frame);
    l
}

fn popup(mtm: MainThreadMarker, items: &[&str], frame: NSRect) -> Retained<NSPopUpButton> {
    let p = NSPopUpButton::initWithFrame_pullsDown(NSPopUpButton::alloc(mtm), frame, false);
    for item in items {
        p.addItemWithTitle(&NSString::from_str(item));
    }
    p
}

/// Muestra el formulario. `None` si el usuario cancela o no hay nada que lanzar.
pub fn run(mtm: MainThreadMarker, project: &Path) -> Option<Form> {
    let mut models = vec!["Predeterminado"];
    models.extend(MODELS.iter().copied());

    let rows = 5.0;
    let (row_h, gap, preview_h, task_h) = (26.0, 8.0, 64.0, 54.0);
    let height = task_h + rows * (row_h + gap) + preview_h + gap;
    let accessory = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(WIDTH, height)),
    );
    let control_x = LABEL_W + 8.0;
    let control_w = WIDTH - control_x;
    let mut y = height;
    let mut next = |h: f64| {
        y -= h + gap;
        y
    };

    let y_task = next(task_h);
    accessory.addSubview(&label(
        mtm,
        "Tarea",
        NSRect::new(
            NSPoint::new(0.0, y_task + task_h - 20.0),
            NSSize::new(LABEL_W, 18.0),
        ),
    ));
    let task = NSTextField::initWithFrame(
        NSTextField::alloc(mtm),
        NSRect::new(
            NSPoint::new(control_x, y_task),
            NSSize::new(control_w, task_h),
        ),
    );
    task.setPlaceholderString(Some(&NSString::from_str("¿Qué tiene que hacer Claude?")));
    task.setUsesSingleLineMode(false);
    accessory.addSubview(&task);

    let mut row = |text: &str| -> NSRect {
        let ry = next(row_h);
        accessory.addSubview(&label(
            mtm,
            text,
            NSRect::new(NSPoint::new(0.0, ry + 4.0), NSSize::new(LABEL_W, 18.0)),
        ));
        NSRect::new(NSPoint::new(control_x, ry), NSSize::new(control_w, row_h))
    };
    let isolation = popup(
        mtm,
        &["Nuevo worktree", "Carpeta actual"],
        row("Aislamiento"),
    );
    // `-w` solo funciona dentro de un repositorio git.
    if !launch::is_git_repo(project) {
        isolation.selectItemAtIndex(1);
    }
    accessory.addSubview(&isolation);
    let name_frame = row("Nombre del worktree");
    let name = NSTextField::initWithFrame(NSTextField::alloc(mtm), name_frame);
    accessory.addSubview(&name);
    let mode = popup(
        mtm,
        &["Plan primero", "Preguntar", "Aceptar ediciones"],
        row("Al empezar"),
    );
    accessory.addSubview(&mode);
    let model = popup(mtm, &models, row("Modelo"));
    accessory.addSubview(&model);
    // Quinta fila: el proyecto, solo informativo.
    let project_frame = row("Proyecto");
    let project_label =
        NSTextField::labelWithString(&NSString::from_str(&project.display().to_string()), mtm);
    project_label.setFont(Some(&NSFont::systemFontOfSize(12.0)));
    project_label.setFrame(NSRect::new(
        NSPoint::new(project_frame.origin.x, project_frame.origin.y + 4.0),
        NSSize::new(control_w, 18.0),
    ));
    accessory.addSubview(&project_label);

    let y_prev = next(preview_h);
    let preview = NSTextField::wrappingLabelWithString(&NSString::from_str(""), mtm);
    preview.setFont(Some(&NSFont::monospacedSystemFontOfSize_weight(11.0, 0.0)));
    preview.setFrame(NSRect::new(
        NSPoint::new(0.0, y_prev),
        NSSize::new(WIDTH, preview_h),
    ));
    preview.setSelectable(true);
    accessory.addSubview(&preview);

    let helper = Helper::alloc(mtm).set_ivars(HelperIvars {
        task: task.clone(),
        name: name.clone(),
        isolation: isolation.clone(),
        mode: mode.clone(),
        model: model.clone(),
        preview,
        launch: RefCell::new(None),
    });
    let helper: Retained<Helper> = unsafe { msg_send![super(helper), init] };

    // Los desplegables avisan por acción; los textos, por notificación.
    for p in [&isolation, &mode, &model] {
        // SAFETY: `refresh:` existe en `Helper`.
        unsafe {
            p.setTarget(Some(&helper));
            p.setAction(Some(sel!(refresh:)));
        }
    }
    let center = NSNotificationCenter::defaultCenter();
    for field in [&task, &name] {
        // SAFETY: `refresh:` existe en `Helper`; la constante es de AppKit.
        unsafe {
            center.addObserver_selector_name_object(
                &helper,
                sel!(refresh:),
                Some(NSControlTextDidChangeNotification),
                Some(field),
            );
        }
    }

    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str("Nuevo agente"));
    alert.setInformativeText(&NSString::from_str(
        "Claude Code trabajará en una pestaña nueva. Podrás aprobar sus permisos desde aquí.",
    ));
    alert.setAccessoryView(Some(&accessory));
    alert.addButtonWithTitle(&NSString::from_str("Lanzar"));
    alert.addButtonWithTitle(&NSString::from_str("Cancelar"));
    *helper.ivars().launch.borrow_mut() = alert.buttons().firstObject();
    // Verificación: `RT_SHEET_PREFILL="tarea"` rellena el campo y
    // `RT_SNAPSHOT_DIR` vuelca el formulario a PNG antes de mostrarlo.
    if let Some(text) = std::env::var("RT_SHEET_PREFILL")
        .ok()
        .filter(|t| !t.is_empty())
    {
        task.setStringValue(&NSString::from_str(&text));
    }
    helper.update();
    alert.window().setInitialFirstResponder(Some(&task));
    if let Some(dir) = std::env::var_os("RT_SNAPSHOT_DIR") {
        let dir = std::path::PathBuf::from(dir);
        let _ = std::fs::create_dir_all(&dir);
        crate::agent_ui::snapshot_png(&accessory, &dir.join("sheet.png").to_string_lossy());
    }

    let result = alert.runModal();
    // SAFETY: se retira el observador que se registró arriba.
    unsafe { center.removeObserver(&helper) };
    if result != NSAlertFirstButtonReturn {
        return None;
    }
    let form = helper.form();
    form.to_spec().validate().is_ok().then_some(form)
}
