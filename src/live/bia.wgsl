// BIA en 3D: material PBR sencillo (color, normales, metal-rugosidad) con las luces de la web.
struct B {
  vp: mat4x4<f32>,
  model: mat4x4<f32>,
  eye: vec4<f32>,
  l1d: vec4<f32>,   // hacia la luz 1 (normalizada)
  l1c: vec4<f32>,   // color * intensidad
  l2d: vec4<f32>,
  l2c: vec4<f32>,
  pl_pos: vec4<f32>, // luz puntual: xyz en el mundo, w = distancia
  pl_col: vec4<f32>, // color * intensidad, w = decaimiento
  ambient: vec4<f32>,
  flat_col: vec4<f32>, // piezas lisas: rgb = color, w = rugosidad
  flags: vec4<f32>,    // x = 1 si lleva texturas
};
@group(0) @binding(0) var<uniform> b: B;
@group(0) @binding(1) var base_tex: texture_2d<f32>;
@group(0) @binding(2) var normal_tex: texture_2d<f32>;
@group(0) @binding(3) var mr_tex: texture_2d<f32>;
@group(0) @binding(4) var smp: sampler;

const PI: f32 = 3.14159265;

struct VIn { @location(0) pos: vec3<f32>, @location(1) nrm: vec3<f32>, @location(2) uv: vec2<f32> };
struct VOut {
  @builtin(position) p: vec4<f32>,
  @location(0) world: vec3<f32>,
  @location(1) nrm: vec3<f32>,
  @location(2) uv: vec2<f32>,
};

@vertex fn vs(v: VIn) -> VOut {
  var o: VOut;
  let w = b.model * vec4<f32>(v.pos, 1.0);
  o.world = w.xyz;
  o.p = b.vp * w;
  // La escala es uniforme: la normal solo necesita la parte lineal del modelo.
  o.nrm = normalize((b.model * vec4<f32>(v.nrm, 0.0)).xyz);
  o.uv = v.uv;
  return o;
}

fn point_atten(d: f32, dist: f32, decay: f32) -> f32 {
  let w = clamp(1.0 - pow(d / dist, 4.0), 0.0, 1.0);
  return (1.0 / max(pow(d, decay), 0.01)) * w * w;
}

fn blinn(n: vec3<f32>, l: vec3<f32>, v: vec3<f32>, f0: vec3<f32>, shin: f32) -> vec3<f32> {
  let h = normalize(l + v);
  return f0 * pow(max(dot(n, h), 0.0), shin) * (shin + 2.0) / (8.0 * PI);
}

fn light(irr: vec3<f32>, n: vec3<f32>, l: vec3<f32>, v: vec3<f32>, diffuse: vec3<f32>, f0: vec3<f32>, shin: f32) -> vec3<f32> {
  return irr * max(dot(n, l), 0.0) * (diffuse / PI + blinn(n, l, v, f0, shin));
}

// Mapeo de tonos ACES (aproximación de Narkowicz), el que aplica por defecto three.js en react-three-fiber.
fn aces(x: vec3<f32>) -> vec3<f32> {
  return clamp((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14), vec3<f32>(0.0), vec3<f32>(1.0));
}

@fragment fn fs(i: VOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
  var n = normalize(i.nrm);
  if (!front) { n = -n; }
  let view = normalize(b.eye.xyz - i.world);
  var albedo = b.flat_col.rgb;
  var rough = b.flat_col.w;
  var metal = 0.0;
  if (b.flags.x > 0.5) {
    albedo = textureSample(base_tex, smp, i.uv).rgb;
    let mr = textureSample(mr_tex, smp, i.uv);
    rough = mr.g;
    // Sin mapa de entorno (como en la web) lo metálico solo refleja las luces puntuales y se ve negro: se suaviza
    // para que las líneas doradas del traje y las alas sigan legibles.
    metal = mr.b * 0.6;
    // Normal mapeada con la base tangente sacada de las derivadas de la pantalla (el GLB no trae tangentes).
    let dp1 = dpdx(i.world);
    let dp2 = dpdy(i.world);
    let du1 = dpdx(i.uv);
    let du2 = dpdy(i.uv);
    let dp2perp = cross(dp2, n);
    let dp1perp = cross(n, dp1);
    let t = dp2perp * du1.x + dp1perp * du2.x;
    let bt = dp2perp * du1.y + dp1perp * du2.y;
    let inv = inverseSqrt(max(max(dot(t, t), dot(bt, bt)), 1e-12));
    let tn = textureSample(normal_tex, smp, i.uv).xyz * 2.0 - 1.0;
    n = normalize(mat3x3<f32>(t * inv, bt * inv, n) * tn);
  }
  rough = clamp(rough, 0.05, 1.0);
  let diffuse = albedo * (1.0 - metal);
  let f0 = mix(vec3<f32>(0.04), albedo, metal);
  let shin = clamp(2.0 / (rough * rough) - 2.0, 2.0, 256.0);

  var col = b.ambient.rgb * diffuse / PI;
  col = col + light(b.l1c.rgb, n, normalize(b.l1d.xyz), view, diffuse, f0, shin);
  col = col + light(b.l2c.rgb, n, normalize(b.l2d.xyz), view, diffuse, f0, shin);
  let to_pl = b.pl_pos.xyz - i.world;
  let d = length(to_pl);
  col = col + light(b.pl_col.rgb * point_atten(d, b.pl_pos.w, b.pl_col.w), n, to_pl / max(d, 0.0001), view, diffuse, f0, shin);
  // Emisión propia de la web (#20222e × 0,3 + la voz): sin ella el traje oscuro se hunde en el fondo.
  if (b.flags.x > 0.5) {
    col = col + vec3<f32>(0.0144, 0.016, 0.0273) * b.flags.y;
  }
  return vec4<f32>(aces(col * (1.0 / 0.6)), 1.0);
}
