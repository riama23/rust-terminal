//! API de control por agentes (v2, **experimental**, protocolo `v0`): un
//! socket Unix por el que un proceso de una terminal de la app (normalmente
//! un agente, a través del comando `rt`) lista sesiones, lee pantalla y
//! bloques y escribe en ellas. Canal distinto del broker de la v1.2.
//!
//! Seguridad (decidida con el usuario): socket en una carpeta privada (0700,
//! socket 0600) y comprobación del usuario del proceso; **un token distinto
//! en cada arranque**, entregado solo a las shells que lanza la app (variables
//! de entorno); **leer funciona con el token; teclear, abrir y cerrar piden
//! permiso en la interfaz** (una vez, o en esta sesión por agente).

pub mod protocol;
pub mod server;
