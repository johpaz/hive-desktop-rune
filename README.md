# Hive de escritorio sobre Rune

Cliente nativo Rust conectado a `hive-gateway-rust`, del proyecto vecino `../hiveSDK-Rust`.
La integración habilita chat, streaming, narraciones, cancelación, sesiones y oficina 3D en vivo.
El coordinador puede consultar a los especialistas de HiveDB mediante `task_delegate`; sus llamadas
al modelo se ejecutan realmente en el SDK y sus estados y relaciones aparecen en la oficina.
HiveLive conecta voz, transcripciones y consultas al coordinador mediante Gemini Live.
El Panel interactivo usa herramientas A2UI nativas. Las conexiones de canales, MCP y
las herramientas externas todavía requieren sus adaptadores de ejecución.

HiveDB es la única fuente de configuración y datos: usuarios, coordinador, proveedores, modelos,
secretos cifrados y conversaciones conservan los contratos del proyecto `hive`.
No se importan bases antiguas de otro formato ni se crea una configuración paralela.

## Vistas y ajustes

Reinicia app y gateway después de actualizar: el contrato de escritorio 4 habilita
Agentes, Canales, Providers, Panel interactivo, Reuniones, API Client, Notas y Cron Jobs.
Ajustes incluye Herramientas, MCP Servers, Skills, Ética, Perfil, Pantalla y audio y
Actualizaciones. El gateway conserva la autenticación de Hive en todas sus rutas.

Los cambios de agentes, modelos, proveedores y perfil se guardan en HiveDB. Las claves
nuevas se cifran con la clave maestra existente. Los canales y MCP muestran su estado
sin inventar conexiones; sus adaptadores externos todavía están pendientes. La vista
de herramientas distingue configuración guardada de disponibilidad de ejecución.

Reuniones usa un modelo STT y su proveedor habilitados en HiveDB, guarda transcripciones
y exporta un informe Markdown. API Client envía solicitudes HTTP/HTTPS reales. Cron
puede encolar y ejecutar tareas textuales con el coordinador, con resultados en Webchat;
los trabajos que necesitan herramientas o canales aún no integrados muestran el error.
El Panel interactivo permite formularios creados por el coordinador y sus acciones
vuelven al SDK. Para usar nuevas herramientas en un hilo que indica configuración
anterior, crea una conversación nueva; se conserva el historial y sus checkpoints.

## Catálogo, visión, voz y Piper

Providers agrupa los modelos por tipo y capacidades: un mismo proveedor como Gemini
puede aparecer en Lenguaje, Imagen y Voz. Imagen distingue análisis de visión de
generación de imágenes. Los predeterminados de visión, transcripción y síntesis se
persisten por separado en HiveDB y conservan el modelo de chat y Gemini 3.8 Live.

El gateway incluye el catálogo oficial revisado el 13/11/2026 (94 entradas; novedades
desde agosto y modelos base de visión/voz). La actualización de HiveDB es aditiva al
abrir el perfil y conserva activaciones y asignaciones; los modelos nuevos quedan
apagados. **CONFIG → Actualizar catálogo** consulta los modelos de la cuenta sin
hacer inferencias. Los modelos sin metadatos publicados se indican en la interfaz.
La disponibilidad puede depender de región, plan y endpoint; Qwen 4 Preview muestra
su requisito de Token Plan y su fecha de finalización.

En Voz, abre **Piper → CONFIG → Consultar estado** para instalar el binario nativo,
descargar y elegir una voz local, ajustar velocidad e iniciar/detener el servicio.
**Probar voz** sintetiza el texto y usa la salida de audio seleccionada. No requiere
clave de API. La instalación automática está disponible en Linux/macOS x64 y ARM64.
El registro de modelos del catálogo no implica implementar todos los transportes de
medios de terceros. La revisión y límites por proveedor están documentados en
[`hiveSDK-Rust/docs/plans/2026-11-13-provider-catalog-and-multimodal.md`](../hiveSDK-Rust/docs/plans/2026-11-13-provider-catalog-and-multimodal.md).

La revisión de [responsividad](docs/plans/2026-11-13-responsive-layout.md) cubre todas
las pantallas y formularios en seis tamaños de ventana. Las columnas se apilan cuando
falta espacio, los identificadores largos se ajustan y los paneles permiten desplazamiento.

## Webchat y conversaciones

Al iniciar se abre el chat web más reciente de HiveDB; se crea uno solo si la lista está vacía.
**Nueva conversación** abre un hilo independiente. El panel lateral permite volver a los
chats anteriores y recuperar sus últimos 40 mensajes, como en el original; en ventanas
estrechas se abre con **Conversaciones**. Para cambiar durante una respuesta, termina o
detén primero el turno. Las pruebas CLI explícitas conservan sus conversaciones aisladas.

El historial textual anterior se usa como contexto inicial cuando todavía no existe una
sesión SDK. Las sesiones existentes conservan sus checkpoints y firmas. Si un turno quedó
interrumpido, detenido por límites o usa otra configuración, su historial sigue visible,
con una indicación para revisar o crear un chat nuevo; no se repiten operaciones pendientes.

**Actividad** agrupa pasos por turno y muestra agente, estado y hora. Cada paso se actualiza
sin duplicar el inicio, escritura y resultado, y las narraciones se recuperan de HiveDB al
volver al chat. Los errores completos se consultan al expandir. Una consulta de texto a un
especialista no se presenta como verificación de acciones externas.

## HiveLive

Reinicia Rune y el gateway y abre **HiveLive**. HiveDB debe tener Gemini habilitado,
su clave cifrada y un modelo realtime habilitado. Al abrir por primera vez esta versión,
se guarda `gemini-3.8-live` como activo en HiveDB y se desmarca el modelo Live anterior.
Las selecciones posteriores del usuario se conservan. Se usa el modelo activo; si no hay uno,
se prefiere el modelo Live del catálogo original y luego el primer modelo compatible.
En HiveLive, **Modelo de voz** permite elegir entre los modelos realtime habilitados del
catálogo. La selección se guarda en HiveDB y se conserva al reiniciar. El modelo, la voz
y el idioma se eligen con la llamada cerrada; el selector espera la confirmación del gateway. El micrófono se abre cuando
Gemini confirma la sesión; cámara y pantalla se comparten solo al activar sus controles.

Al activar cámara o pantalla aparecen recuadros separados con la vista previa y un
control para apagar cada fuente. Las imágenes de vista previa permanecen en la app;
solo se envían a BIA los fotogramas destinados al modelo. La pantalla se considera
iniciada después de recibir una imagen válida; si no llega, se muestra el error.
En Linux/Wayland se usa el portal del sistema y su conexión autorizada a PipeWire.

La llamada usa la conversación seleccionada y bloquea su cambio hasta colgar. BIA puede
consultar al coordinador y sus especialistas de texto, narrar avances y comunicar resultados.
Las tareas aceptadas continúan aunque cuelgues; sus respuestas quedan en el chat y su trabajo
se refleja en la oficina. No ejecutan herramientas externas aún pendientes de migración.
Las transcripciones casuales quedan en HiveDB separadas del historial del webchat para evitar
duplicar las consultas delegadas. El consumo confirmado alimenta el dashboard y su estimación
usa las tarifas del modelo en HiveDB; sin tarifas se indica que no hay precio disponible.

Se admite reconexión únicamente con un identificador de sesión confirmado por Gemini, sin
repetir pedidos anteriores. Los errores se muestran en la vista. Las pruebas automatizadas
usan audio sintético y sockets en memoria; la validación con micrófono y Gemini real queda
pendiente en el entorno de escritorio.

## Dashboard

Reinicia la app y el gateway para habilitar **Dashboard** en el menú. El Centro de Control
se actualiza cada cinco segundos con agentes y estados en vivo, actividad, consumo
registrado en HiveDB y propuestas pendientes del curador. Las métricas CPU/RSS corresponden
al proceso Rust (disponibles mediante `/proc` en Linux); no se presenta un heap JavaScript.

Se recupera la actividad de mensajes de builds Rust anteriores sin duplicarla. El uso de
las nuevas respuestas confirmadas se registra desde esta entrega junto al historial existente
de Hive; no se inventa consumo de turnos Rust antiguos que no tenían registro. Los costos son
estimaciones según tarifas de HiveDB y se señalan como incompletos cuando faltan datos.
Las solicitudes interrumpidas sin respuesta confirmada pueden no estar incluidas. La versión
mostrada es la del gateway local, sin afirmar que se haya comprobado una actualización remota.

## Narraciones y oficina 3D

Reinicia tanto Rune como el gateway después de actualizar: esta versión usa protocolo 2 y no
reutiliza el gateway inicial de protocolo 1. Con tu perfil configurado, abre **Oficina 3D**.
Desde la barra inferior puedes enviar una consulta, ver la respuesta y detener la ejecución sin
salir de la escena. **Actividad** despliega los pasos del coordinador y sus especialistas.

Para observar una delegación, pide al coordinador que consulte a uno de los especialistas que
aparecen en la oficina. Los agentes de catálogo sin propietario también se incluyen, como en Hive.
Los especialistas sin modelo propio heredan la asignación del coordinador. Los agentes desactivados
o archivados no se ejecutan, y se respeta `tools/task_delegate` cuando está desactivado en HiveDB.

Esta entrega admite consultas de texto síncronas y una profundidad de delegación.
Los límites se derivan de `agents.max_iterations` (10 si falta) y del número de especialistas
visibles habilitados. Con I iteraciones y W especialistas, el presupuesto permite I × max(1, W)
llamadas de herramienta y esas consultas más I llamadas del coordinador; su límite local de
iteraciones sigue vigente. Una respuesta puede consultar a varios especialistas en un lote.
La salida de cada modelo reserva
el 15% del contexto efectivo configurado en HiveDB (mínimo entre `context_window` y `num_ctx`
si existen ambos), hasta 32768 tokens; sin contexto configurado usa 4096. El presupuesto
compartido permite una reserva completa de 32768 por llamada de modelo admitida y conserva el consumo real.
Cada llamada de modelo dispone de tres minutos, como en Hive original.
No se comparte un plazo global entre el coordinador y los especialistas. Las conversaciones
anteriores conservan su política durable; abre una nueva después de actualizar el gateway.
Los límites locales del agente siguen vigentes.
El presupuesto compartido del gateway vive durante el turno; los checkpoints del SDK son durables.
Los especialistas todavía no ejecutan archivos, navegador, MCP ni verificaciones de aceptación.

Las narraciones se conservan en `narrationEvents`, y la actividad en `hive_sdk_rust_activity_v1`.
La oficina recibe snapshots con revisión y eventos de trabajo; descarta estados atrasados. Si un
agente tiene varias ejecuciones, terminar una no lo muestra disponible mientras otra sigue activa.
Al perder la conexión se indica que el estado no está confirmado. Reiniciar marca las ejecuciones
interrumpidas para revisión y no repite llamadas al proveedor.

## Arrancar

Para compilar ambos binarios y preparar una copia nueva desde tu sesión de escritorio:

```bash
packaging/linux/run-rust-test.sh          # abre Rune con la copia
packaging/linux/run-rust-test.sh --check  # comprobación sin ventana
```

Detén primero el gateway anterior. El lanzador comprueba el descifrado de la configuración antes
de abrir la app; si la clave disponible no corresponde a los secretos, detiene la prueba y muestra
el diagnóstico. No regenera claves ni borra credenciales. Cada ejecución conserva su copia temporal.

También puedes hacerlo manualmente:

```bash
cargo build --manifest-path ../hiveSDK-Rust/Cargo.toml -p hive-gateway-rust
# Con el gateway anterior detenido, crear una copia nueva y aislada (el destino no debe existir):
../hiveSDK-Rust/target/debug/hive-gateway-rust --prepare-test-home "$HOME/.hive" /tmp/hive-rune-rust-test
unset HIVE_DB_PATH
HIVE_HOME=/tmp/hive-rune-rust-test cargo run -- --check
HIVE_HOME=/tmp/hive-rune-rust-test cargo run  # ventana; usa los proveedores de la copia
cargo run --bin spike3d         # spike de una escena 3D dentro de un elemento de Rune
```
La copia bloquea los archivos redb durante su lectura, rechaza una base abierta y no modifica el
origen. Si los secretos usan el llavero, copia la clave maestra existente a `.master.key` en el
perfil de prueba con permisos 0600; el directorio de prueba tiene permisos 0700.

Solo se reutiliza un gateway Rust compatible en el puerto 18790 que sirva el mismo perfil y base.
Un gateway TypeScript ocupa su puerto pero no se utiliza: Rune busca otro puerto libre.
Los gateways externos no se detienen al cerrar. El sidecar propio se supervisa y se detiene con la app.
`HIVE_GATEWAY_BIN` permite indicar el ejecutable Rust explícitamente; por defecto se busca junto
a la app instalada y luego en `../hiveSDK-Rust/target/{debug,release}`.

El login existente usa bcrypt o Argon2. En Linux necesita `libcrypt` y `libargon2.so.1` respectivamente;
sin ellas falla con un diagnóstico, sin omitir la autenticación. Los proveedores deben tener una
`base_url` válida en HiveDB para los proveedores OpenAI compatibles. Anthropic y Gemini usan los
endpoints predeterminados de sus adaptadores cuando no hay URL, respetando URLs personalizadas.
La asignación explícita del coordinador tiene prioridad sobre el modelo activo por defecto, igual
que en Hive. Se admiten OpenAI compatibles, Anthropic y Gemini nativos; Ollama nativo
todavía no está adaptado. Un turno cancelado o interrumpido conserva el estado incierto del SDK y
requiere reconciliación explícita; para seguir la prueba se puede abrir una conversación nueva
reiniciando la ventana. No se repiten llamadas al modelo automáticamente.

HiveAgents tiene un adaptador Rust específico para su backend GGUF: consulta `/api/status`,
solicita `/api/load` una sola vez cuando corresponde y espera al modelo configurado antes de
inferir con el alias `local`. El contexto de carga sale de HiveDB (`context_window` del modelo,
limitado por `num_ctx` del proveedor cuando existe). Las peticiones del mismo backend se coordinan
dentro del proceso para evitar cambiar de modelo mientras otra respuesta sigue transmitiéndose.
Cancelar detiene la espera y la petición activa; una carga ya aceptada puede continuar en el servidor.
Los fallos identifican al especialista en Actividad y también se imprimen en la consola del gateway.

La tabla siguiente incluye comprobaciones históricas de pantallas; las que requieren voz, tools o
canvas no están disponibles con este gateway inicial. `--check` y `--chat` devuelven código distinto
de cero si el chat falla, falta configuración o vence el timeout.

## Verificar sin ventana
| Comando | Qué hace |
|---|---|
| `cargo test` | La suite completa (no necesita dispositivos ni red externa) |
| `cargo test -- --ignored` | Además abre tu micrófono y altavoz reales (tono corto y bajo) |
| `cargo run -- --check` | Arranca el gateway, comprueba `/health`, lo detiene |
| `cargo run -- --chat "hola"` | Un turno real de chat. **Gasta tokens de tu proveedor** |
| `cargo run -- --a2ui` | Un turno real pidiendo una superficie A2UI y resumiendo lo que llega (`A2UI_DUMP=ruta` guarda los frames). **Gasta una vuelta del modelo** |
| `cargo run -- --a2ui-flow` | Pide el formulario, lo rellena, pulsa «Enviar» y espera al agente. **Gasta dos turnos del modelo** |
| `cargo run -- --tray-check` | Prueba la bandeja por D-Bus: lee el menú, pulsa «Mostrar» (abre la ventana unos segundos) y «Salir» |
| `cargo run -- --kill-check exit\|sigkill\|abort` | Arranca el gateway y sale sin detenerlo, para comprobar que no queda huérfano |
| `cargo run -- --office-check` | Abre la oficina con tu gateway real (solo lectura): agentes, lienzo y resumen de la escena |
| `cargo run -- --voice 15` | Una sesión de voz real de 15 s. **Abre tu micrófono y gasta tokens** |
| `cargo run -- --settings` | Lee proveedores, agentes, canales, herramientas, skills y MCP (solo GET, nunca imprime claves) |
| `cargo run -- --audio` | Lista dispositivos, mide el micrófono 2 s y reproduce un tono |
| `cargo run -- --snap out.png --scene office --size 1100x700 --scale 1` | La oficina 3D con siete abejas en distintos estados |
| `cargo test -- --ignored --nocapture frame_time` | Mide el tiempo por fotograma de la oficina en tu GPU |
| `cargo run -- --snap out.png --scene NOMBRE` | Renderiza una escena a PNG: `a2ui`, `login`, `settings`, `tools`, `markdown`, `dashboard`, `notes`, `cron`, `apiclient`, `a2ui-replay` (con `A2UI_REPLAY=ruta`) |
| `packaging/linux/package.sh` | Compila en release y genera `dist/*.deb` y `dist/*.tar.gz` con el sidecar junto al ejecutable |
| `tar -xzf dist/*.tar.gz && ./hive-agents-rune-*/install.sh` | Instala en `~/.local` sin administrador (lanzador «Hive Agents (Rune)» en el menú); `install.sh --uninstall` lo quita y no toca `~/.hive` |

Bandeja: `--no-tray` la desactiva; cerrar la ventana la manda a la bandeja solo si hay anfitrión (KDE, waybar…; GNOME necesita una extensión) o con `HIVE_CLOSE_TO_TRAY=1`.

Usa `HIVE_HOME=/ruta/temporal` para no tocar `~/.hive`. Las corridas `--snap`, `--script` y `--trace`
no arrancan el gateway.

## Mapa del código
- `src/main.rs`: estado, vistas del chat, acceso y navegación, y los modos de verificación.
- `src/gateway.rs`: ciclo de vida del sidecar (puerto, `/health`, reinicio, cierre).
- `src/chat.rs`: REST + WebSocket `/ws` (turnos en streaming, acceso con correo y contraseña).
- `src/realtime.rs` y `src/voice.rs`: voz en tiempo real (`/realtime`) sobre `crates/hive-media`.
- `src/a2ui/`: `model` (lógica pura), `view` (Rune), `images` y `audio`.
- `src/settings/`: cliente REST y pantalla de ajustes.
- `src/office/`: la oficina 3D: `camera`, `swarm` (enjambre), `model` y `graph` (estado en vivo), `mesh`, `scene` (lista de dibujos), `shaders/*.wgsl`, `gpu` (renderer con bloom) y `stage` (compositor de Rune).
- `src/meeting.rs` y `src/meeting_ui.rs`: reuniones (audio del micrófono y del equipo → WAV por `/meeting-stream`, transcripción en vivo, informe).
- `crates/hive-media`: audio, cámara y pantalla; copia del crate de la app de Tauri, con sus propios avisos de `clippy`.

## Seguridad
- El token de acceso vive en memoria. Solo si marcas «Recordarme» al iniciar sesión se guarda en el llavero del sistema (nunca en un archivo); «Cerrar sesión» en Ajustes lo borra, y un token que el gateway rechaza se olvida solo.
- Las claves de proveedor nunca se muestran (solo la versión enmascarada del servidor) y se borran del campo al enviarlas.
- Las URLs de imágenes y audio las decide el agente: solo http/https, sin redirecciones, con topes de tamaño y de tiempo,
  y sin direcciones locales o privadas (salvo el propio gateway, único que recibe el token).
- Las pruebas de escritura contra el gateway usan siempre un servidor falso, nunca tu configuración real.

## Aprobación de herramientas

Si la política del gateway marca una herramienta como `ask` ([policy.md](../hiveSDK-Rust/docs/policy.md)), el turno se detiene y el gateway manda `approval_request`. El escritorio muestra una tarjeta **sobre la pantalla en la que estés** (no solo en el chat, porque el agente está bloqueado esperando) con la herramienta, el motivo y los argumentos en texto plano, y los botones **Aprobar** y **Denegar**. La respuesta (`{"type":"approval","approvalId","approved"}`) sale por el mismo WebSocket del turno.

- **Nunca se aprueba por omisión.** Vencer el plazo, terminar o detener el turno, perder la conexión, o recibir más de 8 peticiones a la vez (se deniegan las más antiguas) retira la tarjeta sin aprobar nada; el gateway ya denegó al vencer.
- Los argumentos se acotan a 600 caracteres sin cortar ninguno y se muestran tal cual (no se interpretan).
- **En el chat** la pregunta llega además como superficie A2UI en el hilo (la dibuja el gateway y la renderiza el cliente A2UI de siempre); en esa pantalla no se repite la tarjeta. Si estás en otra pantalla, o en otra conversación, la tarjeta sigue siendo la forma de verla. El borrado de la superficie por el gateway retira también la petición.
- Una petición llega aunque hayas cambiado de conversación: el turno que espera es el mismo.
- Escena de captura: `cargo run -- --snap out.png --scene approval --size 1280x760 --scale 1`.

**Probado:** parser y respuesta contra un servidor WebSocket real, y el comportamiento de la interfaz (se ve en otra pantalla, cada botón manda su respuesta una sola vez, caducidad, fin de turno, límite y argumentos); captura renderizada. **No probado:** con un gateway real pidiendo una aprobación de verdad, ni la ventana interactiva con ratón real.

## Dispositivos de audio y vídeo

**Ajustes → Sistema → Pantalla y audio** (también desde el botón de controles de la barra de la llamada en HiveLive) elige:

- **Micrófono** y **salida de la voz**: la lista sale del sistema (PipeWire/ALSA en Linux); «Predeterminado del sistema» sigue al sistema. La entrada `default_*` que publica PipeWire no se repite.
- **Cámara**: solo las que entregan imagen (en Linux un nodo de metadatos de la misma webcam no se ofrece). Sin elegir, se usa la primera. Antes siempre se abría la primera y no se podía cambiar.

La elección se guarda en `~/.config/hive-desktop-rune/audio.json` (sin secretos; un archivo anterior sin `camera` sigue valiendo). **Si hay una llamada en curso, el cambio se aplica al momento**, solo al aparato que cambiaste: el micrófono se reabre, la salida se reabre (lo que sonaba se corta) y la cámara, si está encendida, se apaga y se vuelve a abrir con la elegida. Si el aparato nuevo no abre, se vuelve al del sistema y lo dice, para no dejar la llamada sorda.

**Probado:** la enumeración con la librería real en esta máquina (4 entradas, 2 salidas, 1 cámara), los selectores y su persistencia, que el cambio queda pendiente y se aplica una sola vez, el botón de HiveLive y la captura de la página. **No probado:** el cambio en caliente con una llamada y un aparato reales (abrir la cámara o el micrófono sin que lo pidas enciende su luz), ni con aparatos que se desconectan durante la llamada.

