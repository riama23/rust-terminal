# Issue #12 — Modo de vista del modo review

**Decisión:** VALID (feature-request / enhancement) · **Severidad:** LOW · **Confianza:** media (la imagen adjunta no se descargó)

## Resumen
Se pide que la lista de archivos de Review (⌘⇧R) se pueda ocultar o mostrar como árbol interactivo de carpetas.

## Verificación
- `crates/app/src/review_ui.rs:26-169`: `FileListView` pinta una lista plana. Cada fila es un `FileRow { path, status, comments, selected }` de altura fija (`LIST_ROW_H`), con la ruta recortada por `shorten_head(…, 30)`. La selección es por índice (`file_at`).
- `crates/app/src/review_controller.rs:466,498-510`: el controlador vuelca `r.files` tal cual a filas. No hay agrupación por carpeta, colapsado ni opción para ocultar el panel.
- No hay issues ni PRs previos sobre esto. Las demás issues no se solapan.

## Completitud
| Elemento | Estado |
|---|---|
| Descripción | Presente, ambigua: pide dos cosas (ocultar lista / árbol) |
| Pasos de reproducción | Vacíos (es normal en una petición) |
| Diagnóstico | Presente |
| Imagen de referencia | Presente (no verificada) |
| Texto del panel pegado | Ruido ajeno a la issue (conversación de otra tarea) |

## Alcance sugerido
1. **Árbol:** modelo puro y con tests (en `agent/review.rs` o un módulo nuevo): `Vec<FileDiff>` → nodos con profundidad y plegado. La vista pinta nodos en lugar de `FileRow`, y `file_at` pasa de índice de fila a índice de archivo.
2. **Ocultar la lista:** alternar el ancho de `FileListView` (atajo o botón). Hay que añadirlo a `keybindings::ACTIONS` y `OPTIONS` si es configurable.
3. Persistir el estado de plegado/visibilidad en la sesión de Review (no en config, salvo que se quiera).
4. Un `FileRow` con `depth`/`kind` mantiene las reglas del proyecto: colores con `tok::…()` y etiquetas en los setters.

## Etiquetas sugeridas
`enhancement`, `review`

## Borrador de respuesta
Gracias. Confirmado: hoy la lista de Review es plana (`FileListView`) y no se puede ocultar ni agrupar por carpetas. Propongo dos pasos: (1) árbol plegable de carpetas con los contadores de comentarios agregados por carpeta, y (2) poder ocultar el panel. ¿Quieres ambos o prefieres empezar por el árbol?
