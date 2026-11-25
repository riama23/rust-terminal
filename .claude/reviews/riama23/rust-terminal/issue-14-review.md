# Issue Review: #14 — Fallo: el plan no se carga el sidebar

**Reviewed**: 2026-11-25
**Author**: riama23
**Type**: bug (lectura de «plan» demasiado estrecha)
**Decision**: VALID
**Severity**: MEDIUM
**Confidence**: likely

## Summary
El reportero ve «Sin plan todavía.» en el panel derecho mientras Claude Code ya ha mostrado un plan. El código solo rellena el plan desde `TodoWrite`; un plan de modo plan (`ExitPlanMode`) o las tareas nuevas (`TaskCreate`/`TaskUpdate`) no llegan nunca a `todos`.

## Evidence
- Modo: lectura estática del checkout local, HEAD 5201035. La puerta de limpieza NO se cumple (hay cambios rastreados en Cargo.toml, README.md, scripts/bundle.sh…) y el reportero usaba 665d062; conclusiones sin verificar en ejecución.
- `crates/app/src/agent/model.rs:250-256`: `self.todos` solo se asigna con `PostToolUse` + `tool_name == "TodoWrite"`. El comentario admite «formato sin verificar contra una sesión real».
- `model.rs:108-130` (`parse_todos`): solo lee `tool_input.todos`; devuelve vacío si no existe.
- `model.rs:184`: `ExitPlanMode` solo cambia el estado a `PlanReady`; el texto del plan (`tool_input.plan`) no se guarda.
- `agent_ui.rs:581-582`: pinta «Sin plan todavía.» si `todos` está vacío.
- El texto del issue muestra un plan en prosa (Fases/Riesgos) tras «Plan aprobado»: es un plan de modo plan, no una lista de tareas. Además, la sesión estaba en `auto mode`.
- Confirmado por el autor: `claude` se escribió a mano en la terminal. `rt-integration.zsh:85-102,191` añade `--settings` con los hooks solo si no hay un `claude` propio (alias o función) y `RT_CLAUDE_SETTINGS` es legible; si no, la sesión queda `limited` (sin hooks, sin plan posible). El diagnóstico no dice cuál fue; lo más probable es que hubiera hooks y fallara la lectura del plan.

## Completeness
| Item | Present |
|---|---|
| Reproduction steps | parcial (sin pasos explícitos) |
| Expected behavior | parcial (implícito) |
| Actual behavior | sí |
| Version | sí (665d062) |
| Environment | sí |

## Related
Sin duplicados. #13 es otra petición, no relacionada. Sin PR que lo corrija.

## Suspicious Content
None

## Suggested Labels
El repositorio no tiene etiquetas definidas; sin sugerencia.

## Draft Reply
Gracias por el informe. Lo he revisado: el panel «PLAN» solo se alimenta de las llamadas a `TodoWrite`. Un plan del modo plan (`ExitPlanMode`, texto en `tool_input.plan`) o las tareas nuevas (`TaskCreate`/`TaskUpdate`) no se leen, así que el panel se queda en «Sin plan todavía.». Propuesta: guardar el texto del plan de `ExitPlanMode` y soportar las herramientas de tareas, con tests. Para descartar la otra causa, ¿en la sidebar la sesión aparecía como «En marcha» (sin hooks) o con un estado normal? ¿Tienes un alias o función `claude` en tu `.zshrc`?
