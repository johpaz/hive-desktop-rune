const PI: f32 = 3.14159265;

// smoothstep que admite bordes invertidos (e0 > e1), como GLSL en la práctica; el de WGSL no los define.
fn ss(e0: f32, e1: f32, x: f32) -> f32 {
  let t = clamp((x - e0) / (e1 - e0), 0.0, 1.0);
  return t * t * (3.0 - 2.0 * t);
}

fn hash1(n: f32) -> f32 {
  return fract(sin(n) * 43758.5453123);
}
