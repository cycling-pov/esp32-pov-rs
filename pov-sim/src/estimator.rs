use std::time::Duration;

use crate::state::{NUM_SPOKES, SpokePos};
use bevy::prelude::*;
use pov_algs::Angle;

#[derive(Debug, Resource)]
pub struct PositionEstimator {
    pub pos: pov_algs::filters::PositionEstimator<{ NUM_SPOKES }>,
    spokes: [SpokePos; NUM_SPOKES],
}

impl PositionEstimator {
    const OFFSET: Angle = Angle::from_radians(Angle::CIRCLE.radians() / NUM_SPOKES as f32);

    pub fn step(&mut self, dt: Duration, tick: Option<usize>) {
        self.pos.step(dt, tick);
        self.update_spokes();
    }

    fn update_spokes(&mut self) {
        let base_pos = self.pos.get_current_pos();
        for (i, s) in self.spokes.iter_mut().enumerate() {
            s.prev = s.pos;
            s.pos = (base_pos + Angle::from_radians((i as f32) * Self::OFFSET.radians()))
                .constrain_circle();
        }
    }

    pub fn has_rotated(&self) -> bool {
        self.has_rotated_spoke(0)
    }

    pub fn get_spoke(&self, spoke: usize) -> &SpokePos {
        &self.spokes[spoke]
    }

    pub fn has_rotated_spoke(&self, spoke: usize) -> bool {
        self.spokes.get(spoke).is_some_and(|x| x.has_rotated())
    }
}

impl Default for PositionEstimator {
    fn default() -> Self {
        let mut val = Self {
            pos: pov_algs::filters::PositionEstimator::<{ NUM_SPOKES }>::default(),
            spokes: [SpokePos::default(); NUM_SPOKES],
        };

        for (i, s) in val.spokes.iter_mut().enumerate() {
            let angle_offset = Angle::from_radians(i as f32 * Self::OFFSET.radians());
            s.pos = (val.pos.get_current_pos() + angle_offset).constrain_circle();
            s.prev = s.pos;
        }

        val
    }
}
