struct VOut {
  @builtin(position) clip: vec4<f32>,
  @location(0) world: vec3<f32>,
  @location(1) nrm: vec3<f32>,
  @location(2) color: vec4<f32>,
  @location(3) p0: vec4<f32>,
};

@vertex fn vs(v: VIn) -> VOut {
  var pos = v.pos;
  let t = g.eye.w;
  let glitch = v.p0.y;
  if (glitch > 0.01) {
    let row = floor(pos.y * 14.0 + t * 18.0);
    let slice = step(0.94, hash1(row));
    pos.x = pos.x + slice * glitch * (hash1(t * 43.0) - 0.5) * 0.45;
    pos.z = pos.z + slice * glitch * (hash1(t * 71.0) - 0.5) * 0.2;
  }
  let w = model_of(v) * vec4<f32>(pos, 1.0);
  var o: VOut;
  o.clip = g.vp * w;
  o.world = w.xyz;
  o.nrm = world_normal(v, v.nrm);
  o.color = v.color;
  o.p0 = v.p0;
  return o;
}

@fragment fn fs(i: VOut) -> @location(0) vec4<f32> {
  let t = g.eye.w;
  let intensity = i.p0.x;
  let glitch = i.p0.y;
  let n = normalize(i.nrm);
  let v = normalize(g.eye.xyz - i.world);
  // Fresnel con la normal en el mundo (la UI web la mezcla con una vista en otro espacio: aquí es la correcta).
  let fresnel = pow(1.0 - abs(dot(n, v)), 2.2);
  let scan = 0.82 + 0.18 * sin(i.world.y * 90.0 - t * 6.0);
  var flicker = 0.93 + 0.07 * sin(t * 37.0 + i.world.x * 10.0);
  var col = i.color.rgb;
  if (glitch > 0.01) {
    col = mix(col, vec3<f32>(1.0, 0.28, 0.2), glitch * 0.65);
    flicker = flicker * (0.75 + 0.25 * step(0.5, fract(t * 24.0)));
  }
  let alpha = (0.16 + fresnel * 0.95) * scan * flicker * i.color.a;
  let rgb = col * (intensity * (0.45 + fresnel * 1.7));
  return vec4<f32>(rgb, alpha);
}
