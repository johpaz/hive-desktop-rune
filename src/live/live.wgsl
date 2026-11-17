// Escenario de HiveLive: panal procedural, halo, retrato de BIA (mezcla «screen») y polen.
struct U {
  res: vec2<f32>,
  t: f32,
  level: f32,
  tint: vec4<f32>,   // color del estado de BIA
  stage: vec4<f32>,  // x: proporción del retrato (ancho/alto), y: alto del retrato (fracción), z: densidad de píxeles, w: balanceo
};
@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var bia: texture_2d<f32>;
@group(0) @binding(2) var smp: sampler;

struct O { @builtin(position) p: vec4<f32>, @location(0) uv: vec2<f32> };

@vertex fn vs_full(@builtin(vertex_index) i: u32) -> O {
  let x = f32((i << 1u) & 2u);
  let y = f32(i & 2u);
  var o: O;
  o.p = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
  o.uv = vec2<f32>(x, y);
  return o;
}

fn lin(c: vec3<f32>) -> vec3<f32> { return pow(c, vec3<f32>(2.2)); }

// Cuadrícula hexagonal: devuelve (distancia al borde de la celda, distancia al centro), ambas en unidades de celda.
fn hexcell(p: vec2<f32>) -> vec2<f32> {
  let r = vec2<f32>(1.0, 1.7320508);
  let h = r * 0.5;
  let a = p - r * floor(p / r) - h;
  let b = (p - h) - r * floor((p - h) / r) - h;
  let g = select(b, a, dot(a, a) < dot(b, b));
  let ag = abs(g);
  let edge = 0.5 - max(ag.x, ag.x * 0.5 + ag.y * 0.8660254);
  return vec2<f32>(edge, length(g));
}

@fragment fn fs_backdrop(i: O) -> @location(0) vec4<f32> {
  let px = i.uv * u.res;
  let cell = 150.0 * u.stage.z;
  let hx = hexcell(px / cell + vec2<f32>(0.0, 0.0));
  let edge = hx.x;       // 0 en el borde, ~0.43 en el centro
  let cen = hx.y;
  // Relleno: ámbar oscuro que se apaga hacia los bordes; más luz arriba, como en la web.
  let top = 1.0 - i.uv.y;
  var col = mix(vec3<f32>(0.34, 0.20, 0.06), vec3<f32>(0.06, 0.04, 0.025), smoothstep(0.02, 0.5, cen));
  col = col * (0.6 + 0.9 * top);
  // Borde: línea ámbar fina con un surco oscuro por dentro.
  let line = smoothstep(0.03, 0.0, edge);
  let groove = smoothstep(0.075, 0.03, edge) * (1.0 - line);
  col = col * (1.0 - 0.6 * groove);
  col = col + vec3<f32>(0.78, 0.50, 0.12) * line * (0.40 + 0.60 * top);
  // Halo azul verdoso detrás de BIA y su anillo, centrados en la figura.
  let c = vec2<f32>(0.5 * u.res.x, 0.62 * u.res.y);
  let d = length(px - c) / (0.55 * u.res.y);
  col = mix(col, vec3<f32>(0.04, 0.12, 0.17), 0.6 * smoothstep(1.0, 0.0, d));
  col = col + u.tint.rgb * 0.05 * (0.4 + u.level) * smoothstep(0.9, 0.0, d);
  // Viñeta.
  let v = distance(i.uv, vec2<f32>(0.5, 0.5));
  col = col * (1.0 - 0.55 * smoothstep(0.35, 0.85, v));
  col = mix(vec3<f32>(0.04, 0.045, 0.06), col, 0.92);
  // Tema crema (tint.w = 1): el mismo panal sobre papel cálido, con la línea ámbar más oscura para que se lea.
  var lc = mix(vec3<f32>(0.99, 0.955, 0.87), vec3<f32>(0.94, 0.87, 0.73), 0.65 * smoothstep(0.02, 0.5, cen));
  lc = lc * (1.0 - 0.07 * groove);
  lc = mix(lc, vec3<f32>(0.78, 0.48, 0.08), line * 0.75);
  lc = mix(lc, vec3<f32>(0.78, 0.90, 0.96), 0.35 * smoothstep(1.0, 0.0, d));
  lc = lc * (1.0 - 0.10 * smoothstep(0.35, 0.85, v));
  col = mix(col, lc, u.tint.w);
  return vec4<f32>(lin(clamp(col, vec3<f32>(0.0), vec3<f32>(1.0))), 1.0);
}

@fragment fn fs_portrait(i: O) -> @location(0) vec4<f32> {
  let px = i.uv * u.res;
  let breathe = 1.0 + 0.004 * sin(u.t * 1.3);
  let h = u.res.y * u.stage.y * breathe;
  let w = h * u.stage.x;
  let origin = vec2<f32>(0.5 * u.res.x - 0.5 * w + u.stage.w, u.res.y - h + 0.04 * u.res.y);
  let q = (px - origin) / vec2<f32>(w, h);
  if (q.x < 0.0 || q.x > 1.0 || q.y < 0.0 || q.y > 1.0) { return vec4<f32>(0.0); }
  var c = textureSample(bia, smp, q).rgb;
  // Se difumina el borde del retrato para que no se vea el recorte, salvo por abajo.
  let m = smoothstep(0.0, 0.10, q.x) * smoothstep(1.0, 0.90, q.x) * smoothstep(0.0, 0.06, q.y);
  c = c * m * (1.0 + u.level * 0.35);
  c = c + u.tint.rgb * 0.04 * u.level * m;
  return vec4<f32>(c, 0.0);
}

fn hash(n: f32) -> f32 { return fract(sin(n * 127.1) * 43758.5453); }

struct P { @builtin(position) p: vec4<f32>, @location(0) q: vec2<f32>, @location(1) a: f32 };

@vertex fn vs_pollen(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> P {
  let id = f32(ii);
  let speed = 0.012 + hash(id + 1.0) * 0.03;
  let x = hash(id + 2.0) + 0.02 * sin(u.t * (0.4 + hash(id + 3.0)) + id);
  let y = 1.0 - fract(hash(id + 4.0) - u.t * speed);
  let size = (1.2 + hash(id + 5.0) * 3.2) * u.stage.z * (1.0 + 0.5 * u.level);
  let corner = vec2<f32>(f32((vi & 1u) * 2u) - 1.0, f32(((vi >> 1u) & 1u) * 2u) - 1.0);
  let px = vec2<f32>(x * u.res.x, y * u.res.y) + corner * size * 2.0;
  var o: P;
  o.p = vec4<f32>(px.x / u.res.x * 2.0 - 1.0, 1.0 - px.y / u.res.y * 2.0, 0.0, 1.0);
  o.q = corner;
  o.a = (0.25 + 0.75 * hash(id + 6.0)) * (0.6 + 0.4 * sin(u.t * 1.7 + id * 2.0));
  return o;
}

@fragment fn fs_pollen(i: P) -> @location(0) vec4<f32> {
  let d = length(i.q);
  let k = smoothstep(1.0, 0.0, d);
  let c = mix(vec3<f32>(0.95, 0.62, 0.12), u.tint.rgb, 0.25) * k * k * i.a;
  return vec4<f32>(lin(c), 0.0);
}
