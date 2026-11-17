# Apertura de HiveLive sin bloquear la interfaz

La primera visita construía `LiveGpu` dentro de `LiveStage::render`, en el hilo que
pinta la ventana. Esa ruta leía/esperaba el GLB de BIA, decodificaba el retrato,
preparaba mipmaps, subía texturas y creaba los pipelines gráficos. La precarga existente
solo adelantaba parte de la lectura del GLB. El catálogo de modelos ya se consultaba
en un hilo independiente y la llamada a Gemini comienza al activar el micrófono.

La medición local inicial registró 803 ms para leer el GLB y 1912 ms para construir
`LiveGpu` completo sin precarga. Son mediciones de esta máquina, no un límite universal.

Ahora se preparan los gráficos en un trabajador y el compositor consulta el resultado
sin esperar. Mientras tanto, Rune dibuja los controles y un estado de preparación.
El reloj de dibujo permanece activo hasta recibir los recursos. Los recursos se
reutilizan en las siguientes visitas mientras siga vigente el mismo dispositivo GPU.
Un fallo se presenta aparte de los errores del proveedor y permite reintentar.
La captura de un solo fotograma conserva la preparación síncrona para exportar la escena
completa; no representa el tiempo de navegación interactiva.

La prueba GPU `live_navigation_first_frame_is_nonblocking_and_reuses_graphics`
ejercita el compositor real: comprueba que el primer frame no espere la construcción,
que los controles existan, que llegue la escena preparada y que el siguiente frame
reutilice sus recursos. Las pruebas de entrega del trabajo cubren un trabajador bloqueado
y un fallo. Se vuelven a verificar el estado de voz y la responsividad del escritorio.
