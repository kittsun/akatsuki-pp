use crate::Mods;

/// Summary struct for a [`Beatmap`](crate::Beatmap)'s attributes.
#[derive(Clone, Debug)]
pub struct BeatmapAttributes {
    /// The approach rate.
    pub ar: f64,
    /// The overall difficulty.
    pub od: f64,
    /// The circle size.
    pub cs: f64,
    /// The health drain rate
    pub hp: f64,
    /// The clock rate with respect to mods.
    pub clock_rate: f64,
}

impl BeatmapAttributes {
    const AR0_MS: f64 = 1800.0;
    const AR5_MS: f64 = 1200.0;
    const AR10_MS: f64 = 450.0;
    const AR_MS_STEP_1: f64 = (Self::AR0_MS - Self::AR5_MS) / 5.0;
    const AR_MS_STEP_2: f64 = (Self::AR5_MS - Self::AR10_MS) / 5.0;

    #[inline]
    pub(crate) fn new(ar: f32, od: f32, cs: f32, hp: f32) -> Self {
        Self {
            ar: ar as f64,
            od: od as f64,
            cs: cs as f64,
            hp: hp as f64,
            clock_rate: 1.0,
        }
    }

    /// Adjusts attributes w.r.t. mods.
    /// AR is further adjusted by its hitwindow.
    /// OD is __not__ adjusted by its hitwindow.
    pub fn mods(self, mods: impl Mods) -> Self {
        if !mods.change_map() {
            return self;
        }

        let clock_rate = mods.speed();
        let multiplier = mods.od_ar_hp_multiplier();

        // AR
        let mut ar = (self.ar * multiplier) as f64;
        let mut ar_ms = if ar <= 5.0 {
            Self::AR0_MS - Self::AR_MS_STEP_1 * ar
        } else {
            Self::AR5_MS - Self::AR_MS_STEP_2 * (ar - 5.0)
        };

        ar_ms = ar_ms.max(Self::AR10_MS).min(Self::AR0_MS);
        ar_ms /= clock_rate;

        ar = if ar_ms > Self::AR5_MS {
            (Self::AR0_MS - ar_ms) / Self::AR_MS_STEP_1
        } else {
            5.0 + (Self::AR5_MS - ar_ms) / Self::AR_MS_STEP_2
        };

        // OD
        let od = (self.od * multiplier).min(10.0);

        // CS
        let mut cs = self.cs;
        if mods.hr() {
            cs *= 1.3;
        } else if mods.ez() {
            cs *= 0.5;
        }
        cs = cs.min(10.0);

        // HP
        let hp = (self.hp * multiplier).min(10.0);

        Self {
            ar,
            od,
            cs,
            hp,
            clock_rate,
        }
    }
}
