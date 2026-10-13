//! Frames between frames, for slow motion. `blend` mixes two frames;
//! `MotionPair` follows each pixel's motion from one frame to the next
//! (dense optical flow, coarse to fine) and draws the frame at any moment
//! between them along that motion, leaning on whichever frame sees a pixel
//! where the other cannot (as at the edges of moving things).
//!
//! Pixels may be in either RGBA or BGRA order; both frames must match.
use image::RgbaImage;
use rayon::prelude::*;

/// The longest edge motion is worked out at; it is then scaled to the
/// frame's size.
const FLOW_EDGE: u32 = 512;
const LEVELS: usize = 5;
const ITERATIONS: usize = 4;
/// Half the window, in pixels, a motion is fitted over.
const RADIUS: usize = 4;

/// `a` and `b` mixed: `t` of the way from `a` to `b`.
pub fn blend(a: &RgbaImage, b: &RgbaImage, t: f32) -> RgbaImage {
    let t = t.clamp(0., 1.);
    let mut out = a.clone();
    out.as_mut()
        .par_chunks_mut(4096)
        .zip(b.as_raw().par_chunks(4096))
        .for_each(|(out, b)| {
            for (o, b) in out.iter_mut().zip(b) {
                *o = (f32::from(*o) * (1. - t) + f32::from(*b) * t).round() as u8;
            }
        });
    out
}

/// A grey image as floats.
#[derive(Clone)]
struct Plane {
    width: usize,
    height: usize,
    data: Vec<f32>,
}

impl Plane {
    fn of(image: &RgbaImage, width: u32, height: u32) -> Self {
        let small = if image.dimensions() == (width, height) {
            image.clone()
        } else {
            image::imageops::resize(image, width, height, image::imageops::FilterType::Triangle)
        };
        let data = small
            .pixels()
            .map(|p| (f32::from(p.0[0]) + f32::from(p.0[1]) * 2. + f32::from(p.0[2])) / 1020.)
            .collect();
        Self {
            width: width as usize,
            height: height as usize,
            data,
        }
    }

    fn at(&self, x: usize, y: usize) -> f32 {
        self.data[y * self.width + x]
    }

    /// Bilinear sample, clamped to the edges.
    fn sample(&self, x: f32, y: f32) -> f32 {
        let x = x.clamp(0., (self.width - 1) as f32);
        let y = y.clamp(0., (self.height - 1) as f32);
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.width - 1), (y0 + 1).min(self.height - 1));
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let top = self.at(x0, y0) * (1. - fx) + self.at(x1, y0) * fx;
        let bottom = self.at(x0, y1) * (1. - fx) + self.at(x1, y1) * fx;
        top * (1. - fy) + bottom * fy
    }

    /// Half the size, blurred first so fine detail cannot alias into
    /// false motion at the coarse levels.
    fn half(&self) -> Self {
        let blurred = self.blurred();
        let (width, height) = ((self.width / 2).max(1), (self.height / 2).max(1));
        let data = (0..height)
            .flat_map(|y| (0..width).map(move |x| (x, y)))
            .map(|(x, y)| blurred.at((x * 2).min(self.width - 1), (y * 2).min(self.height - 1)))
            .collect();
        Self {
            width,
            height,
            data,
        }
    }

    /// A 5-tap binomial blur, across then down.
    fn blurred(&self) -> Self {
        const TAPS: [f32; 5] = [1. / 16., 4. / 16., 6. / 16., 4. / 16., 1. / 16.];
        let (w, h) = (self.width, self.height);
        let pass = |data: &[f32], step: (isize, isize)| -> Vec<f32> {
            (0..h)
                .flat_map(|y| (0..w).map(move |x| (x, y)))
                .map(|(x, y)| {
                    TAPS.iter()
                        .enumerate()
                        .map(|(i, tap)| {
                            let k = i as isize - 2;
                            let xx = (x as isize + k * step.0).clamp(0, w as isize - 1) as usize;
                            let yy = (y as isize + k * step.1).clamp(0, h as isize - 1) as usize;
                            data[yy * w + xx] * tap
                        })
                        .sum()
                })
                .collect()
        };
        let across = pass(&self.data, (1, 0));
        Self {
            width: w,
            height: h,
            data: pass(&across, (0, 1)),
        }
    }
}

/// Motion per pixel, in pixels of its plane.
#[derive(Clone)]
struct Field {
    width: usize,
    height: usize,
    dx: Vec<f32>,
    dy: Vec<f32>,
}

impl Field {
    fn zero(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            dx: vec![0.; width * height],
            dy: vec![0.; width * height],
        }
    }

    /// This field at twice the size, its motions doubled.
    fn doubled(&self, width: usize, height: usize) -> Self {
        let mut out = Self::zero(width, height);
        for y in 0..height {
            for x in 0..width {
                let (sx, sy) = ((x / 2).min(self.width - 1), (y / 2).min(self.height - 1));
                out.dx[y * width + x] = self.dx[sy * self.width + sx] * 2.;
                out.dy[y * width + x] = self.dy[sy * self.width + sx] * 2.;
            }
        }
        out
    }

    /// Bilinear sample of the motion at a point.
    fn sample(&self, x: f32, y: f32) -> (f32, f32) {
        let x = x.clamp(0., (self.width - 1) as f32);
        let y = y.clamp(0., (self.height - 1) as f32);
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.width - 1), (y0 + 1).min(self.height - 1));
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let mix = |v: &[f32]| {
            let top = v[y0 * self.width + x0] * (1. - fx) + v[y0 * self.width + x1] * fx;
            let bottom = v[y1 * self.width + x0] * (1. - fx) + v[y1 * self.width + x1] * fx;
            top * (1. - fy) + bottom * fy
        };
        (mix(&self.dx), mix(&self.dy))
    }

    /// Each motion replaced by the median of its 3x3 neighborhood, which
    /// keeps edges between moving things sharp and drops stray estimates.
    fn median(&self) -> Self {
        let mut out = self.clone();
        let (w, h) = (self.width, self.height);
        let pick = |v: &[f32], x: usize, y: usize| {
            let mut near = [0f32; 9];
            let mut n = 0;
            for yy in y.saturating_sub(1)..=(y + 1).min(h - 1) {
                for xx in x.saturating_sub(1)..=(x + 1).min(w - 1) {
                    near[n] = v[yy * w + xx];
                    n += 1;
                }
            }
            let near = &mut near[..n];
            near.sort_unstable_by(f32::total_cmp);
            near[n / 2]
        };
        out.dx
            .par_chunks_mut(w)
            .zip(out.dy.par_chunks_mut(w))
            .enumerate()
            .for_each(|(y, (dx, dy))| {
                for x in 0..w {
                    dx[x] = pick(&self.dx, x, y);
                    dy[x] = pick(&self.dy, x, y);
                }
            });
        out
    }
}

/// Motion from `a` to `b`: where each pixel of `a` went in `b`.
fn motion(a: &Plane, b: &Plane) -> Field {
    let mut pyramid = vec![(a.clone(), b.clone())];
    while pyramid.len() < LEVELS {
        let (a, b) = pyramid.last().expect("level");
        if a.width < 24 || a.height < 24 {
            break;
        }
        pyramid.push((a.half(), b.half()));
    }
    let coarsest = &pyramid.last().expect("level").0;
    let mut field = Field::zero(coarsest.width, coarsest.height);
    for (index, (a, b)) in pyramid.iter().enumerate().rev() {
        if index + 1 < pyramid.len() {
            field = field.doubled(a.width, a.height);
        }
        for _ in 0..ITERATIONS {
            field = refine(a, b, &field).median();
        }
        field = choose(a, b, &field);
    }
    field
}

/// One Lucas-Kanade step: in a window around each pixel, the motion that
/// best explains the difference between `a` and `b` moved by `field`.
fn refine(a: &Plane, b: &Plane, field: &Field) -> Field {
    let (w, h) = (a.width, a.height);
    // Per pixel: gradient products and the difference after moving.
    let mut terms = vec![[0f32; 5]; w * h];
    terms.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        for (x, term) in row.iter_mut().enumerate() {
            let i = y * w + x;
            let (u, v) = (field.dx[i], field.dy[i]);
            let (bx, by) = (x as f32 + u, y as f32 + v);
            let warped = b.sample(bx, by);
            let gx = (b.sample(bx + 1., by) - b.sample(bx - 1., by)) / 2.;
            let gy = (b.sample(bx, by + 1.) - b.sample(bx, by - 1.)) / 2.;
            let gt = warped - a.at(x, y);
            *term = [gx * gx, gx * gy, gy * gy, gx * gt, gy * gt];
        }
    });
    let sums = box_sums(&terms, w, h, RADIUS);
    let mut out = field.clone();
    out.dx
        .par_chunks_mut(w)
        .zip(out.dy.par_chunks_mut(w))
        .enumerate()
        .for_each(|(y, (dx, dy))| {
            for x in 0..w {
                let [xx, xy, yy, xt, yt] = sums[y * w + x];
                let det = xx * yy - xy * xy;
                // Flat or one-directional windows say little: keep what
                // the coarser level found.
                if det.abs() < 1e-6 {
                    continue;
                }
                let du = (-yy * xt + xy * yt) / det;
                let dv = (xy * xt - xx * yt) / det;
                dx[x] += du.clamp(-2., 2.);
                dy[x] += dv.clamp(-2., 2.);
            }
        });
    out
}

/// Each pixel keeps whichever motion (its own, none, or a neighbor's a few
/// pixels away) best matches its surroundings in the other frame. Window
/// fitting smears a moving thing's motion onto the still background around
/// it; this pulls each side's motion back to its side of the edge.
fn choose(a: &Plane, b: &Plane, field: &Field) -> Field {
    const REACH: isize = 3;
    let (w, h) = (a.width, a.height);
    let mismatch = |x: usize, y: usize, u: f32, v: f32| {
        let mut sum = 0.;
        for j in -1..=1isize {
            for i in -1..=1isize {
                let xx = (x as isize + i).clamp(0, w as isize - 1) as usize;
                let yy = (y as isize + j).clamp(0, h as isize - 1) as usize;
                sum += (a.at(xx, yy) - b.sample(xx as f32 + u, yy as f32 + v)).abs();
            }
        }
        sum
    };
    let mut out = field.clone();
    out.dx
        .par_chunks_mut(w)
        .zip(out.dy.par_chunks_mut(w))
        .enumerate()
        .for_each(|(y, (dx, dy))| {
            for x in 0..w {
                let own = (field.dx[y * w + x], field.dy[y * w + x]);
                let mut best = (mismatch(x, y, own.0, own.1), own);
                let mut consider = |u: f32, v: f32| {
                    let cost = mismatch(x, y, u, v);
                    // A little reluctance to change keeps flat areas calm.
                    if cost < best.0 * 0.95 {
                        best = (cost, (u, v));
                    }
                };
                consider(0., 0.);
                for (i, j) in [(-REACH, 0), (REACH, 0), (0, -REACH), (0, REACH)] {
                    let xx = (x as isize + i).clamp(0, w as isize - 1) as usize;
                    let yy = (y as isize + j).clamp(0, h as isize - 1) as usize;
                    consider(field.dx[yy * w + xx], field.dy[yy * w + xx]);
                }
                dx[x] = best.1.0;
                dy[x] = best.1.1;
            }
        });
    out
}

/// Sums of each term over a square window around every pixel.
fn box_sums(terms: &[[f32; 5]], w: usize, h: usize, radius: usize) -> Vec<[f32; 5]> {
    let mut rows = vec![[0f32; 5]; w * h];
    rows.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        let mut sum = [0f32; 5];
        for x in 0..=radius.min(w - 1) {
            add(&mut sum, &terms[y * w + x], 1.);
        }
        for x in 0..w {
            row[x] = sum;
            if x + radius + 1 < w {
                add(&mut sum, &terms[y * w + x + radius + 1], 1.);
            }
            if x >= radius {
                add(&mut sum, &terms[y * w + x - radius], -1.);
            }
        }
    });
    // Down the columns by running sums: each row adds the one above.
    let mut prefix = rows;
    for y in 1..h {
        let (above, here) = prefix.split_at_mut(y * w);
        let above = &above[(y - 1) * w..];
        for (cell, up) in here[..w].iter_mut().zip(above) {
            add(cell, up, 1.);
        }
    }
    let mut out = vec![[0f32; 5]; w * h];
    out.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        let bottom = (y + radius).min(h - 1);
        for (x, cell) in row.iter_mut().enumerate() {
            *cell = prefix[bottom * w + x];
            if y > radius {
                add(cell, &prefix[(y - radius - 1) * w + x], -1.);
            }
        }
    });
    out
}

fn add(sum: &mut [f32; 5], term: &[f32; 5], sign: f32) {
    for (s, t) in sum.iter_mut().zip(term) {
        *s += t * sign;
    }
}

/// Two frames and the motion between them, worked out once and then drawn
/// at any moment between: slow motion asks for many moments of one pair.
pub struct MotionPair {
    a: RgbaImage,
    b: RgbaImage,
    forward: Field,
    backward: Field,
    scale: f32,
}

impl MotionPair {
    pub fn new(a: RgbaImage, b: RgbaImage) -> Self {
        let (width, height) = a.dimensions();
        let scale = (FLOW_EDGE as f32 / width.max(height) as f32).min(1.);
        let (fw, fh) = (
            ((width as f32 * scale).round() as u32).max(8),
            ((height as f32 * scale).round() as u32).max(8),
        );
        let (pa, pb) = (Plane::of(&a, fw, fh), Plane::of(&b, fw, fh));
        let (forward, backward) = rayon::join(|| motion(&pa, &pb), || motion(&pb, &pa));
        Self {
            a,
            b,
            forward,
            backward,
            scale: fw as f32 / width as f32,
        }
    }

    /// The frame `t` of the way from the first frame to the second.
    pub fn at(&self, t: f32) -> RgbaImage {
        let t = t.clamp(0., 1.);
        let (width, height) = self.a.dimensions();
        let mut out = RgbaImage::new(width, height);
        let scale = self.scale;
        out.as_mut()
            .par_chunks_mut(width as usize * 4)
            .enumerate()
            .for_each(|(y, row)| {
                for x in 0..width as usize {
                    let (fx, fy) = (x as f32 * scale, y as f32 * scale);
                    // The point in each frame that moves onto this one at
                    // `t`, found by following the motion back a few times.
                    // A frame where no point lands here (it was hidden
                    // there) is not trusted for this pixel.
                    let (pa_x, pa_y, miss_a) = trace(&self.forward, fx, fy, t);
                    let (pb_x, pb_y, miss_b) = trace(&self.backward, fx, fy, 1. - t);
                    let (u, v) = self.forward.sample(pa_x, pa_y);
                    let (ru, rv) = self.backward.sample(pa_x + u, pa_y + v);
                    let disagree_a = ((u + ru).powi(2) + (v + rv).powi(2)).sqrt();
                    let (bu, bv) = self.backward.sample(pb_x, pb_y);
                    let (su, sv) = self.forward.sample(pb_x + bu, pb_y + bv);
                    let disagree_b = ((bu + su).powi(2) + (bv + sv).powi(2)).sqrt();
                    let trust_a = (1. - t) / (1. + disagree_a + 4. * miss_a);
                    let trust_b = t / (1. + disagree_b + 4. * miss_b);
                    let total = (trust_a + trust_b).max(1e-6);
                    let from_a = (pa_x / scale, pa_y / scale);
                    let from_b = (pb_x / scale, pb_y / scale);
                    let pa = sample(&self.a, from_a.0, from_a.1);
                    let pb = sample(&self.b, from_b.0, from_b.1);
                    let out = &mut row[x * 4..x * 4 + 4];
                    for c in 0..4 {
                        out[c] = ((pa[c] * trust_a + pb[c] * trust_b) / total)
                            .round()
                            .clamp(0., 255.) as u8;
                    }
                }
            });
        out
    }
}

/// The point `p` whose motion carries it onto `(x, y)` after `t` of it
/// (`p + t * motion(p) = (x, y)`), and how far it misses when none does.
fn trace(field: &Field, x: f32, y: f32, t: f32) -> (f32, f32, f32) {
    let (mut px, mut py) = (x, y);
    for _ in 0..5 {
        let (u, v) = field.sample(px, py);
        (px, py) = (x - t * u, y - t * v);
    }
    let (u, v) = field.sample(px, py);
    let miss = ((px + t * u - x).powi(2) + (py + t * v - y).powi(2)).sqrt();
    (px, py, miss)
}

/// Bilinear sample of a frame's pixel, clamped to its edges.
fn sample(image: &RgbaImage, x: f32, y: f32) -> [f32; 4] {
    let (w, h) = image.dimensions();
    let x = x.clamp(0., (w - 1) as f32);
    let y = y.clamp(0., (h - 1) as f32);
    let (x0, y0) = (x.floor() as u32, y.floor() as u32);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let p = |x, y| image.get_pixel(x, y).0.map(f32::from);
    let (a, b, c, d) = (p(x0, y0), p(x1, y0), p(x0, y1), p(x1, y1));
    std::array::from_fn(|i| {
        let top = a[i] * (1. - fx) + b[i] * fx;
        let bottom = c[i] * (1. - fx) + d[i] * fx;
        top * (1. - fy) + bottom * fy
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A bright square with its own markings over a still, uneven
    /// background (as photographed scenes are), at `x`.
    fn scene(x: f32) -> RgbaImage {
        RgbaImage::from_fn(192, 112, |px, py| {
            let (px, py) = (px as f32, py as f32);
            let (sx, sy) = (px - x, py - 56.);
            let value = if sx.abs() < 18. && sy.abs() < 18. {
                200. + (sx * 0.4).sin() * 25. + (sy * 0.3).cos() * 20.
            } else {
                70. + (px * 0.13 + py * 0.05).sin() * 25.
                    + (py * 0.11).cos() * 20.
                    + (px * py * 0.003).sin() * 10.
            };
            let value = value.clamp(0., 255.) as u8;
            image::Rgba([value, value, value, 255])
        })
    }

    /// Mean difference over the part of the frame the square crosses.
    fn error(a: &RgbaImage, b: &RgbaImage) -> f32 {
        let mut sum = 0.;
        let mut count = 0.;
        for y in 30..82 {
            for x in 25..100 {
                sum +=
                    (f32::from(a.get_pixel(x, y).0[0]) - f32::from(b.get_pixel(x, y).0[0])).abs();
                count += 1.;
            }
        }
        sum / count
    }

    #[test]
    fn motion_draws_the_moment_between_frames() {
        let (a, b) = (scene(50.), scene(74.));
        let truth = scene(62.);
        let pair = MotionPair::new(a.clone(), b.clone());
        let followed = error(&pair.at(0.5), &truth);
        let blended = error(&blend(&a, &b, 0.5), &truth);
        // Following the motion lands the square where it was halfway;
        // a blend shows two faint squares.
        assert!(
            followed < blended * 0.25,
            "motion {followed} vs blend {blended}"
        );
        // The ends are the frames themselves.
        assert!(error(&pair.at(0.), &a) < 1.);
        assert!(error(&pair.at(1.), &b) < 1.);
    }
}
