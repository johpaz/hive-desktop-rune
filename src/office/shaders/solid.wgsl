struct VOut {
  @builtin(position) clip: vec4<f32>,
  @location(0) world: vec3<f32>,
  @location(1) nrm: vec3<f32>,
  @location(2) color: vec4<f32>,
  @location(3) p0: vec4<f32>,
  @location(4) p1: vec4<f32>,
};

@vertex fn vs(v: VIn) -> VOut {
  let w = model_of(v) * vec4<f32>(v.pos, 1.0);
  var o: VOut;
  o.clip = g.vp * w;
  o.world = w.xyz;
  o.nrm = world_normal(v, v.nrm);
  o.color = v.color;
  o.p0 = v.p0;
  o.p1 = v.p1;
  return o;
}

// Atenuación de luz puntual como en three.js: 1/d^decay con una ventana que la anula en `distance`.
fn point_atten(d: f32, dist: f32, decay: f32) -> f32 {
  let w = clamp(1.0 - pow(d / dist, 4.0), 0.0, 1.0);
  return (1.0 / max(pow(d, decay), 0.01)) * w * w;
}

fn blinn(n: vec3<f32>, l: vec3<f32>, v: vec3<f32>, f0: vec3<f32>, shin: f32) -> vec3<f32> {
  let h = normalize(l + v);
  return f0 * pow(max(dot(n, h), 0.0), shin) * (shin + 2.0) / (8.0 * PI);
}

fn point_light(pos: vec4<f32>, color: vec4<f32>, world: vec3<f32>, n: vec3<f32>, v: vec3<f32>, diffuse: vec3<f32>, f0: vec3<f32>, shin: f32) -> vec3<f32> {
  let to_l = pos.xyz - world;
  let d = length(to_l);
  let l = to_l / max(d, 0.0001);
  let irr = color.rgb * point_atten(d, pos.w, color.w) * max(dot(n, l), 0.0);
  return irr * (diffuse / PI + blinn(n, l, v, f0, shin));
}

fn shade(i: VOut, double_sided: bool) -> vec4<f32> {
  var n = normalize(i.nrm);
  let v = normalize(g.eye.xyz - i.world);
  if (double_sided && dot(n, v) < 0.0) { n = -n; }
  let albedo = i.color.rgb;
  let metal = i.p1.x;
  let rough = clamp(i.p1.y, 0.05, 1.0);
  let diffuse = albedo * (1.0 - metal);
  let f0 = mix(vec3<f32>(0.04), albedo, metal);
  let shin = clamp(2.0 / (rough * rough) - 2.0, 2.0, 256.0);

  // Todo en luz lineal; la difusa es albedo/π por la irradiancia, como el material estándar de three.
  var col = g.ambient.rgb * diffuse / PI;

  let ld = normalize(g.dir_to_light.xyz);
  let ndl = max(dot(n, ld), 0.0);
  col = col + g.dir_color.rgb * ndl * (diffuse / PI + blinn(n, ld, v, f0, shin));

  col = col + point_light(g.pl_pos, g.pl_color, i.world, n, v, diffuse, f0, shin);
  if (g.ol_pos.w > 0.0) {
    col = col + point_light(g.ol_pos, g.ol_color, i.world, n, v, diffuse, f0, shin);
  }

  col = col + i.p0.rgb * i.p0.w;

  let fog = ss(g.fog.x, g.fog.y, length(g.eye.xyz - i.world));
  col = mix(col, g.fog_color.rgb, fog);
  return vec4<f32>(col, i.color.a);
}

@fragment fn fs_opaque(i: VOut) -> @location(0) vec4<f32> { return shade(i, false); }
@fragment fn fs_alpha(i: VOut) -> @location(0) vec4<f32> { return shade(i, true); }
