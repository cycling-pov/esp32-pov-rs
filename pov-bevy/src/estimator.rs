use crate::state::{NUM_SPOKES, SpokePos};
use bevy::prelude::*;
use pov_algs::CIRCLE_RADIANS;
use std::ops::Rem;

#[derive(Debug, Resource)]
pub struct PositionEstimator {
    pub pos: pov_algs::filters::PositionEstimator<{ NUM_SPOKES }>,
    spokes: [SpokePos; NUM_SPOKES],
}

impl PositionEstimator {
    const OFFSET: f32 = CIRCLE_RADIANS / NUM_SPOKES as f32;

    pub fn step(&mut self, dt: f32, tick: Option<usize>) {
        self.pos.step(dt, tick);
        self.update_spokes();
    }

    //pub fn contains(&self, x: f32) -> bool {
    //    self.spokes.iter().any(|s| s.contains(x))
    //}

    fn update_spokes(&mut self) {
        let base_pos = self.pos.get_current_pos();
        for (i, s) in self.spokes.iter_mut().enumerate() {
            s.prev = s.pos;
            s.pos = (base_pos + (i as f32) * Self::OFFSET).rem(CIRCLE_RADIANS);
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
            s.pos = (val.pos.get_current_pos() + (i as f32) * Self::OFFSET).rem(CIRCLE_RADIANS);
            s.prev = s.pos;
        }

        val
    }
}
