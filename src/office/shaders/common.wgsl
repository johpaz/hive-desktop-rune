struct Globals {
  vp: mat4x4<f32>,
  eye: vec4<f32>,          // xyz, w = tiempo
  fog: vec4<f32>,          // x = cerca, y = lejos
  fog_color: vec4<f32>,
  ambient: vec4<f32>,      // rgb * intensidad
  dir_to_light: vec4<f32>, // hacia la luz, normalizada
  dir_color: vec4<f32>,    // rgb * intensidad
  pl_pos: vec4<f32>,       // xyz, w = distancia
  pl_color: vec4<f32>,     // rgb * intensidad, w = decaimiento
  ol_pos: vec4<f32>,       // luz del oráculo: xyz, w = distancia (0 = apagada)
  ol_color: vec4<f32>,     // rgb * intensidad, w = decaimiento
  view: vec4<f32>,         // ancho y alto en píxeles, densidad de píxeles
};
@group(0) @binding(0) var<uniform> g: Globals;

struct VIn {
  @location(0) pos: vec3<f32>,
  @location(1) nrm: vec3<f32>,
  @location(2) m0: vec4<f32>,
  @location(3) m1: vec4<f32>,
  @location(4) m2: vec4<f32>,
  @location(5) m3: vec4<f32>,
  @location(6) color: vec4<f32>,
  @location(7) p0: vec4<f32>,
  @location(8) p1: vec4<f32>,
};

fn model_of(v: VIn) -> mat4x4<f32> {
  return mat4x4<f32>(v.m0, v.m1, v.m2, v.m3);
}

// Matriz normal sin invertir: la cofactor de la 3x3 es proporcional a la inversa transpuesta,
// y la normal se renormaliza después.
fn world_normal(v: VIn, n: vec3<f32>) -> vec3<f32> {
  let c0 = v.m0.xyz; let c1 = v.m1.xyz; let c2 = v.m2.xyz;
  let cof = mat3x3<f32>(cross(c1, c2), cross(c2, c0), cross(c0, c1));
  return normalize(cof * n);
}
