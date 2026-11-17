//! Transformaciones 4×4 (columna mayor, como WGSL): trasladar, rotar, escalar y componer.

use super::camera::{mul, Mat4, Vec3};

pub const IDENTITY: Mat4 = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];

pub fn translate(t: Vec3) -> Mat4 {
    let mut m = IDENTITY;
    m[12] = t[0];
    m[13] = t[1];
    m[14] = t[2];
    m
}

pub fn scale(s: Vec3) -> Mat4 {
    let mut m = IDENTITY;
    m[0] = s[0];
    m[5] = s[1];
    m[10] = s[2];
    m
}

pub fn rot_x(a: f32) -> Mat4 {
    let (s, c) = a.sin_cos();
    let mut m = IDENTITY;
    m[5] = c;
    m[6] = s;
    m[9] = -s;
    m[10] = c;
    m
}

pub fn rot_y(a: f32) -> Mat4 {
    let (s, c) = a.sin_cos();
    let mut m = IDENTITY;
    m[0] = c;
    m[2] = -s;
    m[8] = s;
    m[10] = c;
    m
}

pub fn rot_z(a: f32) -> Mat4 {
    let (s, c) = a.sin_cos();
    let mut m = IDENTITY;
    m[0] = c;
    m[1] = s;
    m[4] = -s;
    m[5] = c;
    m
}

/// Rotación con los ángulos de Euler de three.js (orden XYZ): primero X, luego Y, luego Z sobre el objeto.
pub fn euler(x: f32, y: f32, z: f32) -> Mat4 {
    mul(&mul(&rot_x(x), &rot_y(y)), &rot_z(z))
}

/// Compone de izquierda a derecha: `chain(&[a, b, c])` aplica primero `c`, luego `b`, luego `a` al vértice.
pub fn chain(ms: &[Mat4]) -> Mat4 {
    ms.iter().fold(IDENTITY, |acc, m| mul(&acc, m))
}

pub fn apply(m: &Mat4, p: Vec3) -> Vec3 {
    [
        m[0] * p[0] + m[4] * p[1] + m[8] * p[2] + m[12],
        m[1] * p[0] + m[5] * p[1] + m[9] * p[2] + m[13],
        m[2] * p[0] + m[6] * p[1] + m[10] * p[2] + m[14],
    ]
}

/// Orienta el eje +Z local hacia `dir` (para que la abeja mire hacia donde vuela), con el eje Y del mundo como arriba.
pub fn look_along(dir: Vec3) -> Mat4 {
    let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
    if len < 1e-6 {
        return IDENTITY;
    }
    let z = [dir[0] / len, dir[1] / len, dir[2] / len];
    let mut x = [z[2], 0.0, -z[0]]; // up × z con up = (0,1,0)
    let xl = (x[0] * x[0] + x[2] * x[2]).sqrt();
    if xl < 1e-6 {
        x = [1.0, 0.0, 0.0]; // mirando recto arriba o abajo
    } else {
        x = [x[0] / xl, 0.0, x[2] / xl];
    }
    let y = [
        z[1] * x[2] - z[2] * x[1],
        z[2] * x[0] - z[0] * x[2],
        z[0] * x[1] - z[1] * x[0],
    ];
    [
        x[0], x[1], x[2], 0.0, y[0], y[1], y[2], 0.0, z[0], z[1], z[2], 0.0, 0.0, 0.0, 0.0, 1.0,
    ]
}

/// Orienta el eje +Y local hacia `dir` (para los rayos, que son cilindros a lo largo de Y).
pub fn align_y(dir: Vec3) -> Mat4 {
    let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
    if len < 1e-6 {
        return IDENTITY;
    }
    let y = [dir[0] / len, dir[1] / len, dir[2] / len];
    // Un eje cualquiera no paralelo a `y` para armar la base.
    let helper = if y[1].abs() > 0.99 {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    let cross = |a: Vec3, b: Vec3| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let norm = |v: Vec3| {
        let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        [v[0] / l, v[1] / l, v[2] / l]
    };
    let x = norm(cross(helper, y));
    let z = cross(x, y);
    [
        x[0], x[1], x[2], 0.0, y[0], y[1], y[2], 0.0, z[0], z[1], z[2], 0.0, 0.0, 0.0, 0.0, 1.0,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec3, b: Vec3) -> bool {
        a.iter().zip(b.iter()).all(|(x, y)| (x - y).abs() < 1e-4)
    }

    #[test]
    fn translate_and_scale_move_and_stretch_points() {
        assert!(close(
            apply(&translate([1.0, 2.0, 3.0]), [0.0, 0.0, 0.0]),
            [1.0, 2.0, 3.0]
        ));
        assert!(close(
            apply(&scale([2.0, 3.0, 4.0]), [1.0, 1.0, 1.0]),
            [2.0, 3.0, 4.0]
        ));
    }

    #[test]
    fn rotations_are_right_handed() {
        let h = std::f32::consts::FRAC_PI_2;
        assert!(close(apply(&rot_z(h), [1.0, 0.0, 0.0]), [0.0, 1.0, 0.0]));
        assert!(close(apply(&rot_x(h), [0.0, 1.0, 0.0]), [0.0, 0.0, 1.0]));
        assert!(close(apply(&rot_y(h), [0.0, 0.0, 1.0]), [1.0, 0.0, 0.0]));
    }

    #[test]
    fn chain_applies_the_last_matrix_first() {
        let m = chain(&[translate([10.0, 0.0, 0.0]), scale([2.0, 2.0, 2.0])]);
        assert!(
            close(apply(&m, [1.0, 0.0, 0.0]), [12.0, 0.0, 0.0]),
            "primero escala, luego traslada"
        );
    }

    #[test]
    fn euler_applies_x_then_y_then_z_to_the_object() {
        let h = std::f32::consts::FRAC_PI_2;
        let m = euler(h, h, 0.0);
        // El orden XYZ de three equivale a R = Rx·Ry·Rz sobre el vértice: primero Rz, luego Ry, luego Rx.
        let expected = apply(&rot_x(h), apply(&rot_y(h), [1.0, 0.0, 0.0]));
        assert!(close(apply(&m, [1.0, 0.0, 0.0]), expected));
    }

    #[test]
    fn look_along_points_local_z_at_the_direction() {
        for dir in [
            [1.0, 0.0, 0.0],
            [0.0, 0.0, -1.0],
            [1.0, 1.0, 1.0],
            [0.0, 1.0, 0.0],
            [0.0, -1.0, 0.0],
        ] {
            let m = look_along(dir);
            let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
            let z = apply(&m, [0.0, 0.0, 1.0]);
            assert!(
                close(z, [dir[0] / len, dir[1] / len, dir[2] / len]),
                "{dir:?} -> {z:?}"
            );
            let x = apply(&m, [1.0, 0.0, 0.0]);
            let y = apply(&m, [0.0, 1.0, 0.0]);
            let dot = |a: Vec3, b: Vec3| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
            assert!(
                dot(x, y).abs() < 1e-4 && dot(x, z).abs() < 1e-4 && dot(y, z).abs() < 1e-4,
                "base ortogonal para {dir:?}"
            );
        }
    }

    #[test]
    fn look_along_a_zero_vector_is_the_identity() {
        assert_eq!(look_along([0.0, 0.0, 0.0]), IDENTITY);
    }

    #[test]
    fn align_y_points_local_y_at_the_direction() {
        for dir in [
            [1.0, 0.0, 0.0],
            [0.0, 3.0, 0.0],
            [0.0, -1.0, 0.0],
            [1.0, 2.0, -3.0],
        ] {
            let m = align_y(dir);
            let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
            assert!(
                close(
                    apply(&m, [0.0, 1.0, 0.0]),
                    [dir[0] / len, dir[1] / len, dir[2] / len]
                ),
                "{dir:?}"
            );
            // Una base ortonormal conserva la longitud.
            let v = apply(&m, [1.0, 1.0, 1.0]);
            assert!(
                ((v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt() - 3.0f32.sqrt()).abs() < 1e-4
            );
        }
        assert_eq!(align_y([0.0, 0.0, 0.0]), IDENTITY);
    }
}
