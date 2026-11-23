# Verificación manual de la v1.3

Lista de comprobación de la v1.3: notificaciones con acciones, badge del Dock, icono en la barra de menús y atajo global ⌃⌥N. Los tests automáticos cubren la política de avisos (`agent/alerts.rs`: cuándo avisar, una vez por petición o por turno, y cómo se interpretan las acciones), que una notificación vieja no puede contestar otra petición (`Hub::pending_request`) y las constantes de Carbon. Lo demás necesita el sistema real.

**Criterio de salida:** responder a un permiso desde la notificación con la app en segundo plano, y ver el contador correcto en el Dock y en la barra de menús.

## Ya comprobado

Con un agente simulado contra el broker en vivo, leyendo lo que AppKit tiene puesto realmente (`RT_SNAPSHOT_DIR` escribe `status.txt`):

- Sin permisos pendientes: sin badge en el Dock; icono de la barra de menús «◐ 1».
- Con un permiso pendiente: badge «1»; icono «! 1»; menú «! Agente de prueba — Necesita tu atención · Bash», separador y «Nuevo agente…  (⌃⌥N)».
- ⌃⌥N se registra sin error (`hotkey=registrado`).

## Pendiente de comprobar a mano

- [ ] **Permiso de notificaciones.** macOS no se lo concede a `rust-terminal.app` por defecto: la app escribe «notificaciones no permitidas en Ajustes del Sistema». Actívalo en Ajustes del Sistema › Notificaciones.
- [ ] Con la app en segundo plano, un permiso muestra una notificación con el nombre de la sesión (y la rama), la orden exacta y tres acciones: *Permitir una vez*, *Enviar respuesta* (con campo de texto) y *Abrir*.
- [ ] *Permitir una vez* desde la notificación hace que Claude Code siga sin tocar la app.
- [ ] *Enviar respuesta* deniega y Claude recibe el texto como motivo. Con el campo vacío no decide nada y abre la sesión.
- [ ] *Abrir* (y un clic en la notificación) trae la app y el panel del agente al frente.
- [ ] Contestar una notificación **vieja** (la petición ya se contestó en la sesión) no hace nada.
- [ ] Con la app **en primer plano** no salen notificaciones.
- [ ] Al terminar un turno o fallar, sale una notificación «Terminó el turno» o «Error», una sola por turno.
- [ ] El badge del Dock cuenta sesiones que necesitan al usuario (no salida sin leer) y desaparece al contestar.
- [ ] El icono de la barra de menús aparece con el primer agente, desaparece con el último, y elegir un agente en su menú lleva la app a ese panel.
- [ ] **⌃⌥N desde otra aplicación** abre el formulario de nuevo agente. Si otra app usa ese atajo, el registro falla y se avisa por stderr.

## Puntos abiertos

- El aviso de «terminado» no incluye aún el número de archivos ni el resultado de los tests (el spec lo pide): necesita el diff de la v1.4.
- Con `cargo run` (sin `.app`) no hay notificaciones: salen por stderr, como antes.
