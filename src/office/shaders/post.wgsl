struct P {
  texel: vec2<f32>,    // 1 / tamaño de la textura de origen
  threshold: f32,
  smoothing: f32,
  intensity: f32,
  vig_offset: f32,
  vig_dark: f32,
  radius: f32,
};
@group(0) @binding(0) var tex0: texture_2d<f32>;
@group(0) @binding(1) var tex1: texture_2d<f32>;
@group(0) @binding(2) var smp: sampler;
@group(0) @binding(3) var<uniform> p: P;

struct VOut { @builtin(position) clip: vec4<f32>, @location(0) uv: vec2<f32> };

@vertex fn vs(@builtin(vertex_index) i: u32) -> VOut {
  let x = f32((i << 1u) & 2u);
  let y = f32(i & 2u);
  var o: VOut;
  o.clip = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
  o.uv = vec2<f32>(x, y);
  return o;
}

fn luma(c: vec3<f32>) -> f32 { return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722)); }

// Promedio de cuatro muestras bilineales en las esquinas del texel: reduce a la mitad sin parpadeo.
fn box4(t: texture_2d<f32>, uv: vec2<f32>, d: vec2<f32>) -> vec3<f32> {
  return (textureSample(t, smp, uv + vec2<f32>(-d.x, -d.y)).rgb + textureSample(t, smp, uv + vec2<f32>(d.x, -d.y)).rgb
        + textureSample(t, smp, uv + vec2<f32>(-d.x, d.y)).rgb + textureSample(t, smp, uv + vec2<f32>(d.x, d.y)).rgb) * 0.25;
}

// Umbral de luminancia y primera reducción.
@fragment fn fs_thr(i: VOut) -> @location(0) vec4<f32> {
  let c = box4(tex0, i.uv, p.texel);
  let mask = ss(p.threshold, p.threshold + p.smoothing, luma(c));
  return vec4<f32>(c * mask, 1.0);
}

@fragment fn fs_down(i: VOut) -> @location(0) vec4<f32> {
  return vec4<f32>(box4(tex0, i.uv, p.texel), 1.0);
}

// Ampliación con filtro de tienda 3x3 del nivel pequeño (tex0), mezclada con el nivel actual (tex1):
// `radius` es el peso de lo ampliado, así que cada nivel conserva la energía en vez de sumarla.
@fragment fn fs_up(i: VOut) -> @location(0) vec4<f32> {
  let d = p.texel;
  var c = textureSample(tex0, smp, i.uv).rgb * 4.0;
  c = c + (textureSample(tex0, smp, i.uv + vec2<f32>(-d.x, 0.0)).rgb + textureSample(tex0, smp, i.uv + vec2<f32>(d.x, 0.0)).rgb
         + textureSample(tex0, smp, i.uv + vec2<f32>(0.0, -d.y)).rgb + textureSample(tex0, smp, i.uv + vec2<f32>(0.0, d.y)).rgb) * 2.0;
  c = c + textureSample(tex0, smp, i.uv + vec2<f32>(-d.x, -d.y)).rgb + textureSample(tex0, smp, i.uv + vec2<f32>(d.x, -d.y)).rgb
        + textureSample(tex0, smp, i.uv + vec2<f32>(-d.x, d.y)).rgb + textureSample(tex0, smp, i.uv + vec2<f32>(d.x, d.y)).rgb;
  let up = c / 16.0;
  return vec4<f32>(mix(textureSample(tex1, smp, i.uv).rgb, up, p.radius), 1.0);
}

// Imagen + resplandor, luego viñeta (la fórmula del efecto Vignette de postprocessing).
@fragment fn fs_comp(i: VOut) -> @location(0) vec4<f32> {
  var c = textureSample(tex0, smp, i.uv).rgb + textureSample(tex1, smp, i.uv).rgb * p.intensity;
  let d = distance(i.uv, vec2<f32>(0.5));
  c = c * ss(0.8, p.vig_offset * 0.799, d * (p.vig_dark + p.vig_offset));
  return vec4<f32>(c, 1.0);
}
