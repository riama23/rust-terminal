# Verificación manual de la v1.4

Lista de comprobación de la v1.4: Review (diff y comentarios en lote), inspector y barra de terminado. Los tests automáticos cubren el parser de diffs (`agent/diff.rs`), la capa de git contra repositorios reales (`agent/git.rs`), el modelo de la revisión, el saneado de comentarios y la regla de cuándo se puede enviar (`agent/review.rs`), y el análisis del plan (`agent/model.rs`).

**Criterio de salida:** revisar los cambios de un agente, dejar comentarios en varias líneas y devolverlos a la sesión en un único envío.

## Desviaciones del spec

- **El diff no es `base...HEAD`.** Claude Code normalmente no hace commit, y ese diff saldría vacío justo cuando hay trabajo que revisar. Se compara el **árbol de trabajo contra la base común** (`git merge-base HEAD <base>`), con los archivos nuevos sin seguimiento (hasta 200, de 1 MiB como máximo) y respetando `.gitignore`.
- **El plan sale de `TodoWrite`**, no de `TaskCreated`/`TaskCompleted` (esos son de subagentes). **Sin verificar contra una sesión real**: en `claude -p` esa herramienta no existe y la captura interactiva no fue concluyente. Si el formato no encaja, la sección «PLAN» queda en «Sin plan todavía».
- **El inspector solo aparece si cabe**: la columna central conserva 560 pt (spec). Con la sidebar abierta, hacen falta 1120 pt de ventana; sin ella, 856.

## Ya comprobado

- **Envío a un Claude real:** un mensaje de varias líneas, pegado entre corchetes y con el Intro aparte y 400 ms después, llega como un único mensaje de usuario y Claude actúa sobre él (en la prueba se puso a buscar el archivo del que hablaban los comentarios). Se probó con `RT_REVIEW_AUTOSEND`.
- **Un comentario hostil no inyecta nada:** ESC, retorno de carro y Ctrl+C se eliminan; dentro del pegado solo quedan los dos ESC del envoltorio.
- **El envío se rechaza** salvo con el agente en «Terminado»: con un permiso o un plan pendientes, el texto los contestaría; trabajando, se mezclaría con su turno.
- **Volcados a PNG (`RT_SNAPSHOT_DIR`):** la ventana de Review (cabecera, lista A/M/D con contador, diff con numeración antigua y nueva, signos `+`/`−`, comentarios intercalados), el inspector con datos reales (cambios, contexto 4 % de la status line, rama) y la barra de terminado tras un turno real.
- La barra de terminado aparece sola al terminar el turno (calcula el diff en segundo plano) y el contexto del inspector viene de la status line.

## Pendiente de comprobar a mano

- [ ] ⌘⇧R (y «Revisar cambios del agente» en el menú Shell) abre la revisión del agente del panel con el foco; en una pestaña que no es de un agente avisa.
- [ ] Al pasar el ratón por una línea del diff aparece el «+»; un clic abre el cuadro del comentario, que se añade a la revisión **sin enviarse**.
- [ ] El «×» de un comentario lo borra; el contador de la lista de archivos y el botón «Enviar N comentarios a Claude» se actualizan.
- [ ] «Enviar» con el agente trabajando o con un permiso pendiente muestra el motivo y no envía nada.
- [ ] «Enviar» con el agente terminado teclea el mensaje en su pestaña, lo envía y deja la revisión vacía.
- [ ] «Actualizar» recalcula el diff y avisa de los comentarios que ya no apuntan a una línea existente.
- [ ] Archivos binarios, borrados, renombrados y nuevos se muestran con su etiqueta y sin romper la vista.
- [ ] Un diff muy grande no congela la interfaz.
- [ ] En una ventana ancha (≥ 1120 pt con sidebar) aparece el inspector; al estrecharla desaparece sin dejar un hueco.
- [ ] La sección PLAN muestra la lista de tareas de un agente real (aquí se comprueba el formato de `TodoWrite`).
- [ ] La barra de terminado lleva a la revisión con su botón y con ⌘⇧R.
- [ ] VoiceOver lee los archivos, las líneas del diff (con su signo) y los comentarios.

## Fuera de la v1.4 (sigue pendiente del spec A3)

- **Descartar worktree** y **Confirmar y abrir PR** (y *Open PR* en la barra de terminado): borran trabajo o publican en un remoto; piden un diseño con confirmaciones y no se han hecho.
- Vista dividida del diff (el spec la ofrece como alternativa a la unificada).
- Cola de revisiones y aviso de límite blando con muchos agentes.
- El aviso de «terminado» del sistema no incluye todavía archivos ni tests.

## Notas de desarrollo

- `RT_OPEN_REVIEW=1`, `RT_REVIEW_SELECT=n`, `RT_REVIEW_DEMO=1` (comentarios de ejemplo) y `RT_REVIEW_AUTOSEND=1` son ayudas de verificación y solo existen en compilaciones debug.
- Un test del núcleo, `term::tests::unhandled_sequences_are_recorded`, falla de forma intermitente (1 de cada ~12 ejecuciones de `cargo test`). Es anterior a esta versión (`crates/core`, sin tocar) y probablemente comparte estado global con otros tests.

## Cierre de la v1.4: PR y descartar worktree

- [ ] La barra de terminado tiene «Abrir PR…» junto a «Revisar cambios», y la cabecera de Review tiene «Abrir PR…» y «Descartar worktree…».
- [ ] «Abrir PR…» **pide confirmación con las órdenes exactas** (`git push -u origin <rama>` y `gh pr create --fill …`) y avisa de los archivos sin commit, que no entran en el PR. Sin rama propia o sin commits nuevos, explica por qué no se puede. Al terminar enseña la URL con «Abrir en el navegador».
- [ ] «Descartar worktree…» solo funciona con worktrees de `.claude/worktrees`, no con el proyecto. Confirma, enseña cuántos archivos sin commit se pierden y **conserva la rama**. No deja descartar con el agente trabajando.
- Sin comprobar a mano: los clics y el flujo completo contra GitHub.

## Árbol de archivos y panel ocultable (issue #12)

La lista de archivos de Review es ahora un árbol de carpetas (`review_tree`, lógica pura con tests) y se puede ocultar.

- [ ] Las carpetas salen primero, con sus archivos sangrados debajo; una cadena de carpetas con un solo hijo (`crates/app/src`) ocupa una sola fila.
- [ ] Clic en una carpeta la pliega (▸) o la despliega (▾). Una carpeta plegada sigue mostrando la suma de comentarios de lo que contiene.
- [ ] Clic en un archivo lo selecciona y enseña su diff; el contador de comentarios del archivo y el de su carpeta coinciden.
- [ ] El botón ◧ de la cabecera oculta la lista y el diff ocupa todo el ancho; al volver a pulsarlo reaparece con la selección y el plegado intactos.
- [ ] Redimensionar la ventana (mínimo 760×420) con la lista visible y oculta no deja huecos ni recorta el diff.
- [ ] Los nombres largos se recortan con «…» y no se salen de la fila ni tapan la píldora de comentarios.
- [ ] Se ve bien con los temas Dusk y Paper (todo sale de `palette::tok`).
- Pendiente: un atajo de teclado para ocultar la lista y la navegación con ↑/↓. Exigen una entrada de menú y encaminar la acción a una ventana que no es la de la terminal.

