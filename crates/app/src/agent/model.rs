//! Modelo de las sesiones de agente: estado a partir de los hooks y orden de
//! la sidebar. Puro (sin AppKit), para poder probarlo a fondo.
//!
//! Orden del spec: *Necesita tu atención*, *Trabajando*, *Error*,
//! *Plan listo*, *Terminado*. Solo *Necesita tu atención* cuenta para el
//! badge del Dock.

use serde_json::Value;

use super::broker::{AgentKey, BrokerEvent, Ended, PermissionRequest};

#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    /// Bloqueado esperando al usuario. `tool` es la herramienta que pide permiso.
    NeedsYou {
        tool: String,
    },
    Working,
    Error,
    /// Propuso un plan y espera aprobación.
    PlanReady,
    /// Terminó el turno.
    Done,
    /// La sesión de Claude Code terminó.
    Ended,
}

impl Status {
    /// Posición en la sidebar: menor, antes.
    fn rank(&self) -> u8 {
        match self {
            Status::NeedsYou { .. } => 0,
            Status::Working => 1,
            Status::Error => 2,
            Status::PlanReady => 3,
            Status::Done => 4,
            Status::Ended => 5,
        }
    }

    /// Texto de la fila (icono y color van aparte: nunca solo color).
    pub fn label(&self) -> String {
        match self {
            Status::NeedsYou { tool } if tool.is_empty() => "Necesita tu atención".into(),
            Status::NeedsYou { tool } => format!("Necesita tu atención · {tool}"),
            Status::Working => "Trabajando".into(),
            Status::Error => "Error".into(),
            Status::PlanReady => "Plan listo · esperando".into(),
            Status::Done => "Terminado".into(),
            Status::Ended => "Cerrada".into(),
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            Status::NeedsYou { .. } => "!",
            Status::Working => "◐",
            Status::Error => "✕",
            Status::PlanReady => "☰",
            Status::Done => "✓",
            Status::Ended => "–",
        }
    }
}

/// Datos de la status line que se muestran en el pie y el inspector.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Telemetry {
    pub model: Option<String>,
    pub context_percent: Option<f64>,
    pub cost_usd: Option<f64>,
    pub version: Option<String>,
    pub branch: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AgentSession {
    pub key: AgentKey,
    pub title: String,
    pub status: Status,
    /// Petición de permiso pendiente (la que muestra la attention bar).
    pub pending: Option<PermissionRequest>,
    pub telemetry: Telemetry,
    /// Orden de creación, para desempatar.
    pub seq: u64,
}

fn str_at<'a>(v: &'a Value, path: &[&str]) -> Option<&'a str> {
    path.iter().try_fold(v, |v, k| v.get(k))?.as_str()
}

fn num_at(v: &Value, path: &[&str]) -> Option<f64> {
    path.iter().try_fold(v, |v, k| v.get(k))?.as_f64()
}

impl AgentSession {
    pub fn new(key: AgentKey, title: String, seq: u64) -> Self {
        AgentSession {
            key,
            title,
            status: Status::Working,
            pending: None,
            telemetry: Telemetry::default(),
            seq,
        }
    }

    /// Aplica un evento del broker. Devuelve `true` si cambió algo visible.
    pub fn apply(&mut self, event: &BrokerEvent) -> bool {
        let before = (
            self.status.clone(),
            self.pending.clone().map(|p| p.id),
            self.telemetry.clone(),
        );
        match event {
            BrokerEvent::Permission(p) => {
                self.status = if p.tool_name == "ExitPlanMode" {
                    Status::PlanReady
                } else {
                    Status::NeedsYou {
                        tool: p.tool_name.clone(),
                    }
                };
                self.pending = Some(p.clone());
            }
            BrokerEvent::PermissionEnded { id, reason, .. } => {
                if self.pending.as_ref().is_some_and(|p| p.id == *id) {
                    self.pending = None;
                    match reason {
                        // Se contestó en la propia sesión: el agente sigue.
                        Ended::ClientGone => self.status = Status::Working,
                        Ended::AgentGone => self.status = Status::Ended,
                        // Se liberó sin decisión: el prompt de Claude Code sigue
                        // en pantalla esperando. La fila mantiene la atención
                        // (la attention bar ya no puede contestar) hasta que
                        // llegue otro evento de la sesión.
                        Ended::TimedOut | Ended::Superseded => {}
                    }
                }
            }
            BrokerEvent::StatusLine { payload, .. } => {
                let t = &mut self.telemetry;
                t.model = str_at(payload, &["model", "display_name"])
                    .map(str::to_owned)
                    .or(t.model.take());
                t.context_percent =
                    num_at(payload, &["context_window", "used_percentage"]).or(t.context_percent);
                t.cost_usd = num_at(payload, &["cost", "total_cost_usd"]).or(t.cost_usd);
                t.version = str_at(payload, &["version"])
                    .map(str::to_owned)
                    .or(t.version.take());
                t.branch = str_at(payload, &["worktree", "branch"])
                    .map(str::to_owned)
                    .or(t.branch.take());
            }
            BrokerEvent::Hook { event, payload, .. } => self.apply_hook(event, payload),
        }
        before
            != (
                self.status.clone(),
                self.pending.clone().map(|p| p.id),
                self.telemetry.clone(),
            )
    }

    fn apply_hook(&mut self, event: &str, payload: &Value) {
        // Mientras hay un permiso pendiente nada de esto lo contradice: solo lo
        // cierra la decisión, el plazo o el fin de la sesión.
        let blocked = self.pending.is_some();
        match event {
            "SessionStart" | "PreToolUse" | "PostToolUse" | "TaskCreated" | "TaskCompleted"
                if !blocked =>
            {
                self.status = Status::Working;
            }
            "Notification" => match str_at(payload, &["notification_type"]) {
                Some("permission_prompt") if !blocked => {
                    self.status = Status::NeedsYou {
                        tool: String::new(),
                    };
                }
                Some("idle_prompt") if !blocked => self.status = Status::Done,
                _ => {}
            },
            "Stop" => {
                self.pending = None;
                self.status = Status::Done;
            }
            "StopFailure" => {
                self.pending = None;
                self.status = Status::Error;
            }
            "SessionEnd" => {
                self.pending = None;
                self.status = Status::Ended;
            }
            _ => {}
        }
    }
}

/// Orden de la sidebar.
pub fn sort(sessions: &mut [AgentSession]) {
    sessions.sort_by_key(|s| (s.status.rank(), s.seq));
}

/// Sesiones que necesitan al usuario (badge del Dock).
pub fn needing_attention(sessions: &[AgentSession]) -> usize {
    sessions
        .iter()
        .filter(|s| matches!(s.status, Status::NeedsYou { .. }))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn session() -> AgentSession {
        AgentSession::new("k".into(), "Tarea".into(), 0)
    }

    fn hook(event: &str, payload: Value) -> BrokerEvent {
        BrokerEvent::Hook {
            agent: "k".into(),
            event: event.into(),
            payload,
        }
    }

    fn perm(id: u64, tool: &str) -> BrokerEvent {
        BrokerEvent::Permission(PermissionRequest {
            id,
            agent: "k".into(),
            session_id: "s".into(),
            tool_name: tool.into(),
            tool_input: json!({"command": "ls"}),
            cwd: "/".into(),
            suggestions: Value::Null,
        })
    }

    fn ended(id: u64, reason: Ended) -> BrokerEvent {
        BrokerEvent::PermissionEnded {
            id,
            agent: "k".into(),
            reason,
        }
    }

    #[test]
    fn empieza_trabajando() {
        assert_eq!(session().status, Status::Working);
    }

    #[test]
    fn un_permiso_pide_atencion_y_guarda_la_peticion() {
        let mut s = session();
        assert!(s.apply(&perm(1, "Bash")));
        assert_eq!(
            s.status,
            Status::NeedsYou {
                tool: "Bash".into()
            }
        );
        assert_eq!(s.pending.as_ref().unwrap().id, 1);
        assert_eq!(s.status.label(), "Necesita tu atención · Bash");
    }

    #[test]
    fn un_plan_pendiente_es_plan_listo_y_sigue_siendo_contestable() {
        let mut s = session();
        s.apply(&perm(1, "ExitPlanMode"));
        assert_eq!(s.status, Status::PlanReady);
        assert!(s.pending.is_some());
    }

    #[test]
    fn si_se_contesta_en_la_sesion_vuelve_a_trabajar() {
        let mut s = session();
        s.apply(&perm(1, "Bash"));
        s.apply(&ended(1, Ended::ClientGone));
        assert_eq!(s.status, Status::Working);
        assert!(s.pending.is_none());
    }

    #[test]
    fn si_se_libera_sin_decision_sigue_pidiendo_atencion_pero_sin_barra() {
        for reason in [Ended::TimedOut, Ended::Superseded] {
            let mut s = session();
            s.apply(&perm(1, "Bash"));
            s.apply(&ended(1, reason));
            assert!(
                s.pending.is_none(),
                "la barra ya no puede contestar ({reason:?})"
            );
            assert!(matches!(s.status, Status::NeedsYou { .. }), "{reason:?}");
            // En cuanto la sesión sigue (el usuario contestó allí), vuelve a trabajar.
            s.apply(&hook("PostToolUse", json!({})));
            assert_eq!(s.status, Status::Working);
        }
    }

    #[test]
    fn si_el_agente_desaparece_la_sesion_se_cierra() {
        let mut s = session();
        s.apply(&perm(1, "Bash"));
        s.apply(&ended(1, Ended::AgentGone));
        assert_eq!(s.status, Status::Ended);
    }

    #[test]
    fn el_fin_de_una_peticion_antigua_no_toca_la_actual() {
        let mut s = session();
        s.apply(&perm(2, "Bash"));
        assert!(!s.apply(&ended(1, Ended::TimedOut)));
        assert_eq!(s.pending.as_ref().unwrap().id, 2);
    }

    #[test]
    fn un_hook_de_actividad_no_apaga_la_atencion_mientras_hay_permiso() {
        let mut s = session();
        s.apply(&perm(1, "Bash"));
        s.apply(&hook("PreToolUse", json!({})));
        s.apply(&hook(
            "Notification",
            json!({"notification_type": "idle_prompt"}),
        ));
        assert!(matches!(s.status, Status::NeedsYou { .. }));
    }

    #[test]
    fn stop_termina_el_turno_y_limpia_lo_pendiente() {
        let mut s = session();
        s.apply(&perm(1, "Bash"));
        s.apply(&hook("Stop", json!({})));
        assert_eq!(s.status, Status::Done);
        assert!(s.pending.is_none());
    }

    #[test]
    fn stop_failure_es_error_y_session_end_cierra() {
        let mut s = session();
        s.apply(&hook("StopFailure", json!({})));
        assert_eq!(s.status, Status::Error);
        s.apply(&hook("SessionEnd", json!({})));
        assert_eq!(s.status, Status::Ended);
    }

    #[test]
    fn un_turno_nuevo_vuelve_a_trabajar() {
        let mut s = session();
        s.apply(&hook("Stop", json!({})));
        s.apply(&hook("PreToolUse", json!({})));
        assert_eq!(s.status, Status::Working);
    }

    #[test]
    fn la_notificacion_de_permiso_sin_hook_de_permiso_tambien_pide_atencion() {
        let mut s = session();
        s.apply(&hook(
            "Notification",
            json!({"notification_type": "permission_prompt"}),
        ));
        assert_eq!(
            s.status,
            Status::NeedsYou {
                tool: String::new()
            }
        );
        assert_eq!(s.status.label(), "Necesita tu atención");
    }

    #[test]
    fn la_status_line_actualiza_la_telemetria_y_conserva_lo_que_falta() {
        let mut s = session();
        s.apply(&BrokerEvent::StatusLine {
            agent: "k".into(),
            payload: json!({
                "model": {"display_name": "Opus"}, "context_window": {"used_percentage": 42.5},
                "cost": {"total_cost_usd": 0.31}, "version": "2.1.330",
                "worktree": {"branch": "worktree-x"}
            }),
        });
        assert_eq!(s.telemetry.model.as_deref(), Some("Opus"));
        assert_eq!(s.telemetry.context_percent, Some(42.5));
        assert_eq!(s.telemetry.branch.as_deref(), Some("worktree-x"));
        // `used_percentage` puede ser null al principio: no borra el último valor.
        s.apply(&BrokerEvent::StatusLine {
            agent: "k".into(),
            payload: json!({"context_window": {"used_percentage": null}}),
        });
        assert_eq!(s.telemetry.context_percent, Some(42.5));
        assert_eq!(s.telemetry.model.as_deref(), Some("Opus"));
    }

    #[test]
    fn aplicar_algo_que_no_cambia_nada_devuelve_false() {
        let mut s = session();
        assert!(!s.apply(&hook("PreToolUse", json!({}))));
        assert!(!s.apply(&hook("EventoDesconocido", json!({}))));
    }

    #[test]
    fn la_sidebar_ordena_como_el_spec() {
        let mk = |seq, status| AgentSession {
            status,
            seq,
            ..AgentSession::new(format!("k{seq}"), "t".into(), seq)
        };
        let mut v = vec![
            mk(0, Status::Done),
            mk(1, Status::PlanReady),
            mk(2, Status::Working),
            mk(
                3,
                Status::NeedsYou {
                    tool: "Bash".into(),
                },
            ),
            mk(4, Status::Error),
            mk(5, Status::Working),
            mk(6, Status::Ended),
        ];
        sort(&mut v);
        let order: Vec<u64> = v.iter().map(|s| s.seq).collect();
        assert_eq!(order, [3, 2, 5, 4, 1, 0, 6]);
    }

    #[test]
    fn el_badge_cuenta_solo_las_que_necesitan_atencion() {
        let mk = |status| AgentSession {
            status,
            ..session()
        };
        let v = [
            mk(Status::NeedsYou {
                tool: "Bash".into(),
            }),
            mk(Status::PlanReady),
            mk(Status::Working),
            mk(Status::NeedsYou {
                tool: String::new(),
            }),
        ];
        assert_eq!(needing_attention(&v), 2);
    }

    #[test]
    fn todos_los_estados_tienen_icono_y_texto() {
        for s in [
            Status::NeedsYou { tool: "x".into() },
            Status::Working,
            Status::Error,
            Status::PlanReady,
            Status::Done,
            Status::Ended,
        ] {
            assert!(!s.icon().is_empty() && !s.label().is_empty(), "{s:?}");
        }
    }
}
