struct VOut {
  @builtin(position) clip: vec4<f32>,
  @location(0) world: vec3<f32>,
  @location(1) color: vec4<f32>,
};

@vertex fn vs(v: VIn) -> VOut {
  let w = model_of(v) * vec4<f32>(v.pos, 1.0);
  var o: VOut;
  o.clip = g.vp * w;
  o.world = w.xyz;
  o.color = v.color;
  return o;
}

// Sin iluminación y opaco (ojos, antenas); recibe la niebla como el MeshBasicMaterial.
@fragment fn fs_opaque(i: VOut) -> @location(0) vec4<f32> {
  let fog = ss(g.fog.x, g.fog.y, length(g.eye.xyz - i.world));
  return vec4<f32>(mix(i.color.rgb, g.fog_color.rgb, fog), 1.0);
}

// Aditivo: color con su opacidad (aros, corona).
@fragment fn fs_add(i: VOut) -> @location(0) vec4<f32> {
  return vec4<f32>(i.color.rgb, i.color.a);
}
