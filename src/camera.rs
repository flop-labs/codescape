//! Orbit/fly camera and the small amount of linear algebra it needs.
//! Matrices are column-major, matching GLSL and Makepad's `Mat4`.

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct V3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

pub const fn v3(x: f32, y: f32, z: f32) -> V3 {
    V3 { x, y, z }
}

impl V3 {
    pub fn add(self, o: V3) -> V3 {
        v3(self.x + o.x, self.y + o.y, self.z + o.z)
    }
    pub fn sub(self, o: V3) -> V3 {
        v3(self.x - o.x, self.y - o.y, self.z - o.z)
    }
    pub fn scale(self, s: f32) -> V3 {
        v3(self.x * s, self.y * s, self.z * s)
    }
    pub fn dot(self, o: V3) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }
    pub fn cross(self, o: V3) -> V3 {
        v3(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }
    pub fn len(self) -> f32 {
        self.dot(self).sqrt()
    }
    pub fn norm(self) -> V3 {
        self.scale(1.0 / self.len().max(1e-12))
    }
    pub fn lerp(self, o: V3, t: f32) -> V3 {
        self.add(o.sub(self).scale(t))
    }
}

pub type M4 = [f32; 16];

pub fn mul(a: &M4, b: &M4) -> M4 {
    let mut r = [0.0; 16];
    for c in 0..4 {
        for row in 0..4 {
            r[c * 4 + row] = (0..4).map(|k| a[k * 4 + row] * b[c * 4 + k]).sum();
        }
    }
    r
}

pub fn transform(m: &M4, p: V3) -> [f32; 4] {
    [
        m[0] * p.x + m[4] * p.y + m[8] * p.z + m[12],
        m[1] * p.x + m[5] * p.y + m[9] * p.z + m[13],
        m[2] * p.x + m[6] * p.y + m[10] * p.z + m[14],
        m[3] * p.x + m[7] * p.y + m[11] * p.z + m[15],
    ]
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub target: V3,
    pub dist: f32,
    /// Rotation about +y; 0 looks toward -z (text reads upright).
    pub yaw: f32,
    /// Elevation above the ground plane, radians.
    pub pitch: f32,
}

pub const FOV_Y: f32 = 50.0 * std::f32::consts::PI / 180.0;

pub struct View {
    pub eye: V3,
    pub forward: V3,
    pub right: V3,
    pub up: V3,
    pub view_proj: M4,
    /// Pixels per world unit at distance 1 along the view axis.
    pub focal_px: f32,
    pub size: (f32, f32),
}

impl Camera {
    pub fn eye(&self) -> V3 {
        let (cp, sp) = (self.pitch.cos(), self.pitch.sin());
        self.target
            .add(v3(self.yaw.sin() * cp, sp, self.yaw.cos() * cp).scale(self.dist))
    }

    pub fn ground_right(&self) -> V3 {
        v3(self.yaw.cos(), 0.0, -self.yaw.sin())
    }

    pub fn ground_forward(&self) -> V3 {
        v3(-self.yaw.sin(), 0.0, -self.yaw.cos())
    }

    pub fn view(&self, width: f32, height: f32) -> View {
        let eye = self.eye();
        let f = self.target.sub(eye).norm();
        let s = f.cross(v3(0.0, 1.0, 0.0)).norm();
        let u = s.cross(f);
        #[rustfmt::skip]
        let view: M4 = [
            s.x, u.x, -f.x, 0.0,
            s.y, u.y, -f.y, 0.0,
            s.z, u.z, -f.z, 0.0,
            -s.dot(eye), -u.dot(eye), f.dot(eye), 1.0,
        ];
        // Depth range adapts to altitude so near-ground text keeps precision.
        let near = (self.dist * 0.02).max(0.05);
        let far = self.dist * 60.0 + 5000.0;
        let aspect = width / height.max(1.0);
        let t = 1.0 / (FOV_Y * 0.5).tan();
        let nf = 1.0 / (near - far);
        #[rustfmt::skip]
        let proj: M4 = [
            t / aspect, 0.0, 0.0, 0.0,
            0.0, t, 0.0, 0.0,
            0.0, 0.0, (far + near) * nf, -1.0,
            0.0, 0.0, 2.0 * far * near * nf, 0.0,
        ];
        View {
            eye,
            forward: f,
            right: s,
            up: u,
            view_proj: mul(&proj, &view),
            focal_px: height * 0.5 * t,
            size: (width, height),
        }
    }

    /// World units per screen pixel at the target.
    pub fn units_per_px(&self, height: f32) -> f32 {
        2.0 * self.dist * (FOV_Y * 0.5).tan() / height.max(1.0)
    }

    pub fn lerp(&self, o: &Camera, t: f32) -> Camera {
        let mut dyaw = o.yaw - self.yaw;
        while dyaw > std::f32::consts::PI {
            dyaw -= std::f32::consts::TAU;
        }
        while dyaw < -std::f32::consts::PI {
            dyaw += std::f32::consts::TAU;
        }
        Camera {
            target: self.target.lerp(o.target, t),
            dist: (self.dist.ln() + (o.dist.ln() - self.dist.ln()) * t).exp(),
            yaw: self.yaw + dyaw * t,
            pitch: self.pitch + (o.pitch - self.pitch) * t,
        }
    }
}

impl View {
    /// Ray through a screen point (pixels, origin top-left).
    pub fn ray(&self, px: f32, py: f32) -> V3 {
        let (w, h) = self.size;
        let dx = (px - w * 0.5) / self.focal_px;
        let dy = (h * 0.5 - py) / self.focal_px;
        self.forward
            .add(self.right.scale(dx))
            .add(self.up.scale(dy))
            .norm()
    }

    /// Intersection of the ray through a screen point with the plane `y`.
    pub fn hit_plane(&self, px: f32, py: f32, y: f32) -> Option<V3> {
        let d = self.ray(px, py);
        if d.y.abs() < 1e-6 {
            return None;
        }
        let t = (y - self.eye.y) / d.y;
        (t > 0.0).then(|| self.eye.add(d.scale(t)))
    }

    /// Screen position of a world point, or None when behind the camera.
    pub fn project(&self, p: V3) -> Option<(f32, f32, f32)> {
        let c = transform(&self.view_proj, p);
        if c[3] <= 1e-4 {
            return None;
        }
        let (w, h) = self.size;
        Some((
            (c[0] / c[3] * 0.5 + 0.5) * w,
            (0.5 - c[1] / c[3] * 0.5) * h,
            c[3],
        ))
    }
}
