# Revisión de responsividad del escritorio Rune

Se mantiene el diseño existente. Las páginas se adaptan al espacio útil de la ventana,
con controles accesibles y desplazamiento vertical cuando el contenido lo requiere.

## Cambios

- Texto normal con ajuste de línea y división de identificadores/rutas largos cuando
  no caben. Los campos de edición de una línea conservan su desplazamiento horizontal.
  La marca y los estados de la cabecera se mantienen en una línea; los estados largos
  conservan el texto completo en su ayuda.
- Cabeceras, tarjetas, filtros y botones se dimensionan dentro del espacio de la página.
  Los grupos de acciones pueden pasar a otra fila.
- Providers, Imagen, Voz y Piper: nombres e identificadores largos se ajustan dentro
  de la tarjeta; el interruptor permanece al lado y los modelos conservan sus controles.
- Agentes: ficha y mapa/editor se apilan cuando no caben juntos. El mapa se escala al
  ancho disponible y el formulario nuevo deja de forzar 640 puntos de ancho. El perfil
  y el editor permiten títulos y contenido largos.
- Perfil: identidad y formulario pasan a una columna en ventanas estrechas.
- Reuniones: el historial pasa debajo de la transcripción en ventanas estrechas, con
  desplazamiento; no reduce el formulario de captura a una franja.
- HiveLive: cabecera ajustable, telemetría con desplazamiento, vistas previas acotadas
  al ancho, transcripción con desplazamiento y altura mínima de escenario para ventanas
  bajas. En ese caso la rueda desplaza la página para llegar a los controles.
- Oficina 3D: los controles de cabecera pueden reorganizarse y las etiquetas proyectadas
  conservan su ancho dentro del escenario.
- Chat: burbujas limitadas al ancho del hilo, bloques de código con desplazamiento
  horizontal propio y sugerencias en varias filas. El estado
  vacío se desplaza en ventanas bajas.
- Menú lateral estrecho: se abre como una capa sobre la página, sin comprimir su ancho;
  se cierra con su botón o pulsando fuera.

## Validación

La prueba nativa `responsive_screens_keep_text_and_controls_inside_the_window` recorre
32 vistas/estados en 480×640, 640×360, 640×480, 800×600, 1024×768 y 1440×900: 192 casos.
Incluye Dashboard, Webchat, HiveLive, Agentes y sus formularios, Canales, Providers,
Imagen, Voz, Piper, Oficina 3D, Panel A2UI, Reuniones, API Client, Notas, Cron Jobs,
Herramientas, Skills, MCP, Ética, Perfil, Pantalla/audio, Actualizaciones y acceso.

También incluye paneles desplegados, edición, respuestas HTTP, identificadores largos,
previsualizaciones de cámara/pantalla y el menú lateral abierto. Se inspeccionan los
rectángulos reales de Rune: textos dentro de sus padres, textos dentro de botones y
cabecera, y componentes con clave dentro del ancho de la ventana. El contenido que
continúa debajo se alcanza con desplazamiento vertical.

La prueba `responsive_sidebar_overlays_content_and_closes_without_changing_page_width`
comprueba apertura/cierre y redimensionado continuo de la misma interfaz.
Se ejecutan además regresiones de navegación, categorías, agentes, reuniones, chat y
controles de HiveLive. Las capturas se generan con el renderizador nativo, sin llamadas
al gateway ni a modelos.

Resultado: los 192 casos, la prueba de menú/redimensionado y las regresiones
de navegación y controles pasaron. Se verificaron capturas nativas de Providers,
Agentes y HiveLive. Se compiló `target/debug/hive-desktop-rune`.
