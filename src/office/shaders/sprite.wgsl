struct VOut {
  @builtin(position) clip: vec4<f32>,
  @location(0) uv: vec2<f32>,
  @location(1) color: vec4<f32>,
};

// Un cuadrado que mira siempre a la cámara. Tamaño base en píxeles (p0.x), atenuado con la distancia
// (p0.y / profundidad) y multiplicado por la densidad de píxeles de la pantalla (g.view.z).
@vertex fn vs(v: VIn) -> VOut {
  let center = vec3<f32>(v.m3.x, v.m3.y, v.m3.z);
  var c = g.vp * vec4<f32>(center, 1.0);
  let size_px = v.p0.x * g.view.z * (v.p0.y / max(c.w, 0.1));
  c.x = c.x + v.pos.x * size_px * 2.0 / g.view.x * c.w;
  c.y = c.y + v.pos.y * size_px * 2.0 / g.view.y * c.w;
  var o: VOut;
  o.clip = c;
  o.uv = v.pos.xy + vec2<f32>(0.5);
  o.color = v.color;
  return o;
}

// Un disco con el borde suave.
@fragment fn fs(i: VOut) -> @location(0) vec4<f32> {
  let d = length(i.uv - vec2<f32>(0.5));
  let disc = ss(0.5, 0.05, d);
  return vec4<f32>(i.color.rgb, i.color.a * disc);
}
