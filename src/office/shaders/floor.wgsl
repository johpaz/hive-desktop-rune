struct VOut {
  @builtin(position) clip: vec4<f32>,
  @location(0) world: vec3<f32>,
  @location(1) color: vec3<f32>,
};

@vertex fn vs(v: VIn) -> VOut {
  let w = model_of(v) * vec4<f32>(v.pos, 1.0);
  var o: VOut;
  o.clip = g.vp * w;
  o.world = w.xyz;
  o.color = v.color.rgb;
  return o;
}

fn modf1(x: f32, y: f32) -> f32 { return x - y * floor(x / y); }

fn hex_edge(p_in: vec2<f32>) -> f32 {
  var p = p_in;
  p.x = p.x * 0.57735 * 2.0;
  p.y = p.y + modf1(floor(p.x), 2.0) * 0.5;
  p = abs(fract(p) - vec2<f32>(0.5));
  return abs(max(p.x * 1.5 + p.y, p.y * 2.0) - 1.0);
}

@fragment fn fs(i: VOut) -> @location(0) vec4<f32> {
  let uv = i.world.xz;
  let dist = length(uv);
  let g1 = hex_edge(uv * 0.55);
  let g2 = hex_edge(uv * 2.2);
  let lines = ss(0.065, 0.0, g1) * 0.8 + ss(0.035, 0.0, g2) * 0.035;
  let fade = ss(46.0, 14.0, dist);
  let center_glow = exp(-dist * 0.11) * 0.12;
  let rgb = i.color * (lines + center_glow);
  let alpha = clamp((lines * 0.3 + center_glow) * fade, 0.0, 0.34);
  return vec4<f32>(rgb, alpha);
}
