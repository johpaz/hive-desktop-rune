# Cámara y pantalla de HiveLive

Diagnóstico: los callbacks de captura ya producen JPEG `Preview` y `Model`, pero
`Realtime::video_sink` descarta Preview y no había paneles para dibujarlos. Ahora un
buzón por fuente guarda solo la última imagen local; el hilo UI actualiza Pixels.
Únicamente Model continúa al socket. Las generaciones del buzón rechazan callbacks
viejos al apagar o volver a abrir una fuente. Dos recuadros muestran cámara/pantalla,
imagen pendiente o congelada y control de cierre, sin tapar los controles de cámara 3D.
Los errores de captura se conservan en el mando de HiveLive.

La pantalla antes informaba éxito al iniciar el backend, aunque no entregara imágenes.
Ahora espera un JPEG válido, con 12 segundos de espera después de la autorización;
cierre previo o falta de imágenes devuelve error. La cámara también notifica cuando
su hilo termina por fallos para apagar su estado visible.

Wayland: xcap 0.9.8 conectaba a PipeWire con el remoto por defecto en lugar del fd
concedido por OpenPipeWireRemote, y no esperaba la respuesta de SelectSources.
Se incluye su fuente Apache-2.0 en vendor/xcap con un parche acotado: fd del portal,
espera de SelectSources, duración/cierre de sesión y terminación de la captura al
perder el stream. El parche y su alcance están en vendor/xcap/HIVE-PATCH.md.
Referencia: https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.ScreenCast.html

El modelo Gemini y su selección en HiveDB no cambian con esta corrección.

Validación: 40 pruebas de hive-media aprobadas (dos diagnósticos ya ignorados),
cuatro de HiveLive, dos del buzón de vista previa y la de controles de cámara/pantalla
aprobadas. Compilación y cargo check de la app aprobados con el parche de xcap.
No se activaron dispositivos ni se hicieron llamadas a modelos.
La prueba independiente adicional de xcap no pudo ejecutarse: no es miembro del
workspace y su resolución aislada offline pide la entrada exr que no está en caché.
El backend Wayland compila como dependencia de la app; queda pendiente su prueba
con el portal y dispositivos reales. El gateway del perfil no era accesible en
127.0.0.1:18790: no se verificó ni cambió su modelo activo en HiveDB.
