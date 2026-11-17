struct VOut {
  @builtin(position) clip: vec4<f32>,
  @location(0) uvy: f32,
  @location(1) color: vec3<f32>,
  @location(2) p0: vec4<f32>,
};

@vertex fn vs(v: VIn) -> VOut {
  let w = model_of(v) * vec4<f32>(v.pos, 1.0);
  var o: VOut;
  o.clip = g.vp * w;
  // El cilindro unitario va de y = -0.5 a 0.5: 0 en el origen del rayo, 1 en su destino.
  o.uvy = v.pos.y + 0.5;
  o.color = v.color.rgb;
  o.p0 = v.p0;
  return o;
}

@fragment fn fs(i: VOut) -> @location(0) vec4<f32> {
  let t = g.eye.w;
  let speed = i.p0.x;
  let flow = fract(i.uvy * 3.0 - t * speed);
  let packet = ss(0.0, 0.12, flow) * ss(0.42, 0.12, flow);
  let flow2 = fract(i.uvy * 7.0 - t * speed * 1.7);
  let packet2 = ss(0.0, 0.06, flow2) * ss(0.2, 0.06, flow2);
  let core = 0.3 + packet * 1.6 + packet2 * 0.9;
  let ends = ss(0.0, 0.05, i.uvy) * ss(1.0, 0.95, i.uvy);
  let glow = core * i.p0.y;
  return vec4<f32>(i.color * glow, clamp(glow * 0.6 * ends, 0.0, 1.0));
}
