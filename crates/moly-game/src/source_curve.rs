//! Authored scalar clip coefficients shared by UI and fixture consumers.
//!
//! This is the existing sitemap sampler, lifted without changing its float
//! operations. Do not resample streamed cubics into linear quaternion keys.

#[derive(Clone)]
pub(crate) enum Curve {
    /// For t_i <= t < t_(i+1), dt=t-t_i and value=((a*dt+b)*dt+c)*dt+d.
    /// Before/after the key domain, preserve the corresponding endpoint.
    Cubic(Vec<(f32, [f32; 4])>),
    Const(f32),
}

impl Curve {
    pub(crate) fn sample(&self, t: f32) -> f32 {
        match self {
            Curve::Const(v) => *v,
            Curve::Cubic(keys) => {
                let Some(&(t0, k0)) = keys.first() else { return 0.0 };
                if t <= t0 {
                    return k0[3];
                }
                let &(tl, kl) = keys.last().expect("cubic curve has a key");
                if t >= tl {
                    return kl[3];
                }
                for pair in keys.windows(2) {
                    let (ta, ka) = pair[0];
                    let (tb, _) = pair[1];
                    if t >= ta && t < tb {
                        let dt = t - ta;
                        return ((ka[0] * dt + ka[1]) * dt + ka[2]) * dt + ka[3];
                    }
                }
                kl[3]
            }
        }
    }
}
