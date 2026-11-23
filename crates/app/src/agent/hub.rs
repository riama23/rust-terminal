//! Las sesiones de agente de una ventana y su relación con los paneles.
//! Puro (sin AppKit): la vista lo consulta para pintar la sidebar y la
//! attention bar.

use std::collections::HashMap;

use super::broker::{AgentKey, BrokerEvent, PermissionRequest};
use super::model::{self, AgentSession};
use crate::workspace::PaneId;

#[derive(Default)]
pub struct Hub {
    /// Siempre en el orden de la sidebar.
    sessions: Vec<AgentSession>,
    panes: HashMap<AgentKey, PaneId>,
    next_seq: u64,
}

impl BrokerEvent {
    pub fn agent(&self) -> &str {
        match self {
            BrokerEvent::Hook { agent, .. }
            | BrokerEvent::StatusLine { agent, .. }
            | BrokerEvent::PermissionEnded { agent, .. } => agent,
            BrokerEvent::Permission(p) => &p.agent,
        }
    }
}

impl Hub {
    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    pub fn sessions(&self) -> &[AgentSession] {
        &self.sessions
    }

    pub fn add(&mut self, key: AgentKey, title: String, pane: PaneId) {
        let seq = self.next_seq;
        self.next_seq += 1;
        self.panes.insert(key.clone(), pane);
        self.sessions.push(AgentSession::new(key, title, seq));
        model::sort(&mut self.sessions);
    }

    pub fn remove(&mut self, key: &str) {
        self.panes.remove(key);
        self.sessions.retain(|s| s.key != key);
    }

    /// Aplica un evento del broker. `true` si hay que repintar.
    pub fn apply(&mut self, event: &BrokerEvent) -> bool {
        let Some(session) = self.sessions.iter_mut().find(|s| s.key == event.agent()) else {
            return false;
        };
        let changed = session.apply(event);
        if changed {
            model::sort(&mut self.sessions);
        }
        changed
    }

    pub fn pane_of(&self, key: &str) -> Option<PaneId> {
        self.panes.get(key).copied()
    }

    pub fn key_of_pane(&self, pane: PaneId) -> Option<&AgentKey> {
        self.panes.iter().find(|(_, p)| **p == pane).map(|(k, _)| k)
    }

    pub fn session_of_pane(&self, pane: PaneId) -> Option<&AgentSession> {
        let key = self.key_of_pane(pane)?;
        self.sessions.iter().find(|s| &s.key == key)
    }

    /// La petición de permiso pendiente del agente que vive en este panel.
    pub fn pending_for_pane(&self, pane: PaneId) -> Option<&PermissionRequest> {
        self.session_of_pane(pane)?.pending.as_ref()
    }

    pub fn needing_attention(&self) -> usize {
        model::needing_attention(&self.sessions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::broker::Ended;
    use serde_json::{Value, json};

    fn perm(agent: &str, id: u64) -> BrokerEvent {
        BrokerEvent::Permission(PermissionRequest {
            id,
            agent: agent.into(),
            session_id: "s".into(),
            tool_name: "Bash".into(),
            tool_input: json!({"command": "ls"}),
            cwd: "/".into(),
            suggestions: Value::Null,
        })
    }

    fn hub() -> Hub {
        let mut h = Hub::default();
        h.add("a".into(), "Primera".into(), 10);
        h.add("b".into(), "Segunda".into(), 20);
        h
    }

    #[test]
    fn la_peticion_pertenece_al_panel_del_agente() {
        let mut h = hub();
        assert!(h.apply(&perm("b", 1)));
        assert_eq!(h.pending_for_pane(20).unwrap().id, 1);
        assert!(h.pending_for_pane(10).is_none());
        assert!(h.pending_for_pane(99).is_none());
    }

    #[test]
    fn la_que_necesita_atencion_sube_la_primera() {
        let mut h = hub();
        assert_eq!(h.sessions()[0].key, "a");
        h.apply(&perm("b", 1));
        assert_eq!(h.sessions()[0].key, "b");
        assert_eq!(h.needing_attention(), 1);
    }

    #[test]
    fn eventos_de_agentes_desconocidos_se_ignoran() {
        let mut h = hub();
        assert!(!h.apply(&perm("zzz", 1)));
    }

    #[test]
    fn quitar_un_agente_libera_su_panel() {
        let mut h = hub();
        h.remove("a");
        assert_eq!(h.pane_of("a"), None);
        assert_eq!(h.key_of_pane(10), None);
        assert_eq!(h.sessions().len(), 1);
        h.remove("b");
        assert!(h.is_empty());
    }

    #[test]
    fn terminar_la_peticion_la_retira_de_la_barra() {
        let mut h = hub();
        h.apply(&perm("a", 1));
        h.apply(&BrokerEvent::PermissionEnded {
            id: 1,
            agent: "a".into(),
            reason: Ended::ClientGone,
        });
        assert!(h.pending_for_pane(10).is_none());
        assert_eq!(h.needing_attention(), 0);
    }
}
