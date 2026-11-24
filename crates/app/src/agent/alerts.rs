//! Qué avisos del sistema genera un agente y qué hacer con sus acciones.
//! Puro (sin AppKit ni `UserNotifications`): el pegamento está en `notify.rs`.
//!
//! Reglas del spec: solo se avisa con la app **inactiva**; *Necesita tu
//! atención* avisa una vez por petición; *Terminado* y *Error* avisan una vez
//! por turno (al entrar en ese estado). Las acciones se atan a la petición
//! concreta (`request_id`): contestar una notificación vieja no puede permitir
//! otra cosa distinta.

use super::broker::{AgentKey, Decision};
use super::model::{AgentSession, Status};

/// Identificadores de las acciones de la notificación de permiso.
pub const ACTION_ALLOW: &str = "rt.allow";
pub const ACTION_REPLY: &str = "rt.reply";
pub const ACTION_OPEN: &str = "rt.open";
/// Clic en la notificación (acción por defecto de `UserNotifications`).
pub const ACTION_DEFAULT: &str = "com.apple.UNNotificationDefaultActionIdentifier";
/// Categoría con las tres acciones de un permiso.
pub const CATEGORY_PERMISSION: &str = "rt.permission";
pub const CATEGORY_INFO: &str = "rt.info";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AlertKind {
    /// Un permiso (o un plan) espera al usuario.
    Permission {
        request_id: u64,
    },
    Done,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alert {
    pub kind: AlertKind,
    pub agent: AgentKey,
    pub title: String,
    pub body: String,
}

/// Lo que cambió en una sesión al aplicar un evento.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub status: Status,
    pub pending_id: Option<u64>,
}

impl Snapshot {
    pub fn of(s: &AgentSession) -> Self {
        Snapshot {
            status: s.status.clone(),
            pending_id: s.pending.as_ref().map(|p| p.id),
        }
    }
}

fn heading(s: &AgentSession) -> String {
    match &s.telemetry.branch {
        Some(branch) => format!("{} · {branch}", s.title),
        None => s.title.clone(),
    }
}

/// El aviso que toca tras pasar de `before` al estado actual de `session`.
pub fn alert_for(before: &Snapshot, session: &AgentSession, app_active: bool) -> Option<Alert> {
    if app_active {
        return None;
    }
    let agent = session.key.clone();
    if let Some(req) = &session.pending {
        // Una petición nueva (no la misma que ya se avisó).
        if before.pending_id == Some(req.id) {
            return None;
        }
        let plan = matches!(session.status, Status::PlanReady);
        let detail = req.detail();
        let body = if plan {
            "Claude propone un plan y espera tu aprobación.".to_owned()
        } else if detail.is_empty() {
            req.tool_name.clone()
        } else {
            format!("{}: {detail}", req.tool_name)
        };
        let title = if plan {
            format!("Plan listo · {}", heading(session))
        } else {
            format!("Necesita tu atención · {}", heading(session))
        };
        return Some(Alert {
            kind: AlertKind::Permission { request_id: req.id },
            agent,
            title,
            body,
        });
    }
    match (&before.status, &session.status) {
        (b, Status::Done) if *b != Status::Done => Some(Alert {
            kind: AlertKind::Done,
            agent,
            title: heading(session),
            body: "Terminó el turno · listo para revisar".into(),
        }),
        (b, Status::Error) if *b != Status::Error => Some(Alert {
            kind: AlertKind::Error,
            agent,
            title: format!("Error · {}", heading(session)),
            body: "Claude Code terminó con un error.".into(),
        }),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// *Permitir una vez*.
    AllowOnce,
    /// *Enviar respuesta*: se deniega y el texto va como motivo.
    Deny(Decision),
    /// Llevar al usuario a la sesión (sin decidir nada).
    Open,
}

/// Interpreta la respuesta del usuario a una notificación. Una respuesta de
/// texto vacía no es una decisión: se trata como abrir la sesión.
pub fn parse_action(id: &str, text: Option<&str>) -> Option<Action> {
    match id {
        ACTION_ALLOW => Some(Action::AllowOnce),
        ACTION_REPLY => {
            let text = text.map(str::trim).filter(|t| !t.is_empty());
            Some(match text {
                Some(t) => Action::Deny(Decision::Deny {
                    message: Some(t.to_owned()),
                }),
                None => Action::Open,
            })
        }
        ACTION_OPEN | ACTION_DEFAULT => Some(Action::Open),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::broker::{BrokerEvent, Ended, PermissionRequest};
    use serde_json::{Value, json};

    fn session() -> AgentSession {
        AgentSession::new("k".into(), "Migrar la base".into(), 0)
    }

    fn perm(id: u64, tool: &str) -> BrokerEvent {
        BrokerEvent::Permission(PermissionRequest {
            id,
            agent: "k".into(),
            session_id: "s".into(),
            tool_name: tool.into(),
            tool_input: json!({"command": "pnpm db:migrate"}),
            cwd: "/".into(),
            suggestions: Value::Null,
        })
    }

    fn hook(event: &str) -> BrokerEvent {
        BrokerEvent::Hook {
            agent: "k".into(),
            event: event.into(),
            payload: json!({}),
        }
    }

    fn step(s: &mut AgentSession, e: &BrokerEvent, active: bool) -> Option<Alert> {
        let before = Snapshot::of(s);
        s.apply(e);
        alert_for(&before, s, active)
    }

    #[test]
    fn un_permiso_nombra_la_sesion_la_orden_y_la_peticion() {
        let mut s = session();
        let a = step(&mut s, &perm(7, "Bash"), false).unwrap();
        assert_eq!(a.kind, AlertKind::Permission { request_id: 7 });
        assert_eq!(a.title, "Necesita tu atención · Migrar la base");
        assert_eq!(a.body, "Bash: pnpm db:migrate");
    }

    #[test]
    fn la_rama_acompana_al_titulo() {
        let mut s = session();
        s.apply(&BrokerEvent::StatusLine {
            agent: "k".into(),
            payload: json!({"worktree": {"branch": "worktree-db"}}),
        });
        let a = step(&mut s, &perm(1, "Bash"), false).unwrap();
        assert_eq!(
            a.title,
            "Necesita tu atención · Migrar la base · worktree-db"
        );
    }

    #[test]
    fn con_la_app_activa_no_se_avisa_de_nada() {
        let mut s = session();
        assert_eq!(step(&mut s, &perm(1, "Bash"), true), None);
        assert_eq!(step(&mut s, &hook("Stop"), true), None);
    }

    #[test]
    fn la_misma_peticion_no_avisa_dos_veces() {
        let mut s = session();
        assert!(step(&mut s, &perm(1, "Bash"), false).is_some());
        // Un evento cualquiera mientras sigue pendiente.
        assert_eq!(step(&mut s, &hook("PreToolUse"), false), None);
        // Otra petición distinta sí avisa.
        s.apply(&BrokerEvent::PermissionEnded {
            id: 1,
            agent: "k".into(),
            reason: Ended::ClientGone,
        });
        assert!(step(&mut s, &perm(2, "Bash"), false).is_some());
    }

    #[test]
    fn un_plan_se_anuncia_como_plan() {
        let mut s = session();
        let a = step(&mut s, &perm(3, "ExitPlanMode"), false).unwrap();
        assert!(a.title.starts_with("Plan listo · "), "{}", a.title);
        assert!(a.body.contains("aprobación"));
        assert_eq!(a.kind, AlertKind::Permission { request_id: 3 });
    }

    #[test]
    fn terminado_y_error_avisan_una_vez_por_turno() {
        let mut s = session();
        let a = step(&mut s, &hook("Stop"), false).unwrap();
        assert_eq!(a.kind, AlertKind::Done);
        assert_eq!(
            step(&mut s, &hook("Stop"), false),
            None,
            "ya estaba terminado"
        );
        // Nuevo turno y nuevo final: vuelve a avisar.
        step(&mut s, &hook("PreToolUse"), false);
        assert!(step(&mut s, &hook("Stop"), false).is_some());
        let e = step(&mut s, &hook("StopFailure"), false).unwrap();
        assert_eq!(e.kind, AlertKind::Error);
        assert_eq!(e.title, "Error · Migrar la base");
    }

    #[test]
    fn la_telemetria_y_la_actividad_no_avisan() {
        let mut s = session();
        assert_eq!(step(&mut s, &hook("PreToolUse"), false), None);
        assert_eq!(step(&mut s, &hook("SessionStart"), false), None);
    }

    #[test]
    fn las_acciones_se_interpretan() {
        assert_eq!(parse_action(ACTION_ALLOW, None), Some(Action::AllowOnce));
        assert_eq!(parse_action(ACTION_OPEN, None), Some(Action::Open));
        assert_eq!(parse_action(ACTION_DEFAULT, None), Some(Action::Open));
        assert_eq!(
            parse_action(ACTION_REPLY, Some("  usa otra ruta  ")),
            Some(Action::Deny(Decision::Deny {
                message: Some("usa otra ruta".into())
            }))
        );
        assert_eq!(parse_action("otra", None), None);
    }

    #[test]
    fn responder_sin_texto_no_es_una_decision() {
        for text in [None, Some(""), Some("   \n")] {
            assert_eq!(
                parse_action(ACTION_REPLY, text),
                Some(Action::Open),
                "{text:?}"
            );
        }
    }
}
